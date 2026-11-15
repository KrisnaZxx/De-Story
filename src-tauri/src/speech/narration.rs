use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio::process::Command;
use uuid::Uuid;

use crate::db::repo::{
    active_project_job, clips_of, get_asset, get_clip, get_project, get_secret, get_settings,
    insert_asset, insert_job, patch_clip, touch_project, update_asset_meta, ClipRow,
    NewAsset,
};
use crate::db::DbPool;
use crate::jobs::{JobManager, QueueName, TaskCtx};
use crate::paths::{asset_abs_path, ffmpeg_path, user_data_dir};
use crate::services::audio::probe_duration_ms;
use crate::services::{elevenlabs, gemini};
use crate::speech::align::{spoken_words, strip_voice_tags, WordTiming};
use crate::speech::cuts::{choose_cuts, SilenceInterval};
use crate::speech::captions::{time_script, CaptionExecution};

const GEMINI_PART_SEC: f64 = 240.0;
const ELEVEN_PART_CHARS: usize = 2400;

#[derive(Debug, Clone)]
struct Segment {
    clip: ClipRow,
    file: PathBuf,
    duration_ms: i64,
    words: Vec<WordTiming>,
    estimated: bool,
}

async fn run_ffmpeg_command(args: &[&str], resource_dir: Option<&Path>) -> Result<String, String> {
    let bin = ffmpeg_path(resource_dir);
    let mut cmd = Command::new(&bin);
    cmd.args(["-hide_banner", "-nostats"])
        .args(args);

    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(0x08000000);
    }

    let out = cmd.output().await.map_err(|e| format!("FFmpeg error ({:?}): {e}", bin))?;
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    if !out.status.success() {
        return Err(format!("FFmpeg failed: {stderr}"));
    }
    Ok(stderr)
}

async fn detect_silences(file: &Path, resource_dir: Option<&Path>) -> Result<Vec<SilenceInterval>, String> {
    let p_str = file.to_string_lossy();
    let log = run_ffmpeg_command(
        &["-i", &p_str, "-af", "silencedetect=noise=-38dB:d=0.25", "-f", "null", "-"],
        resource_dir,
    )
    .await?;

    let mut out = Vec::new();
    let mut cur_start: Option<f64> = None;

    for line in log.lines() {
        if let Some(pos) = line.find("silence_start:") {
            let s_str = line[pos + 14..].trim().split_whitespace().next().unwrap_or("");
            if let Ok(v) = s_str.parse::<f64>() {
                cur_start = Some(v.max(0.0));
            }
        }
        if let Some(pos) = line.find("silence_end:") {
            let s_str = line[pos + 12..].trim().split_whitespace().next().unwrap_or("");
            if let Ok(end_v) = s_str.parse::<f64>() {
                if let Some(st) = cur_start.take() {
                    out.push(SilenceInterval { start: st, end: end_v });
                }
            }
        }
    }

    Ok(out)
}

fn estimate_narration_ms(narration: &str) -> i64 {
    let words = narration.split_whitespace().count();
    ((words as i64 * 400 + 800) as i64).max(1500)
}

fn group_clips(clips: &[ClipRow], provider: &str) -> Vec<Vec<ClipRow>> {
    if provider == "speko" { return clips.iter().map(|c| vec![c.clone()]).collect(); }
    let mut out = Vec::new();
    let mut cur = Vec::new();
    let mut size: f64 = 0.0;

    for c in clips {
        let s = if provider == "elevenlabs" {
            c.narration.len() as f64
        } else {
            (estimate_narration_ms(&c.narration) as f64) / 1000.0
        };
        let limit = if provider == "elevenlabs" {
            ELEVEN_PART_CHARS as f64
        } else {
            GEMINI_PART_SEC
        };

        if !cur.is_empty() && size + s > limit {
            out.push(cur);
            cur = Vec::new();
            size = 0.0;
        }
        cur.push(c.clone());
        size += s;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

async fn cut_audio(
    src: &Path,
    start: f64,
    end: f64,
    dst: &Path,
    resource_dir: Option<&Path>,
) -> Result<(), String> {
    let s_str = format!("{:.3}", start);
    let e_str = format!("{:.3}", end);
    let src_str = src.to_string_lossy();
    let dst_str = dst.to_string_lossy();

    run_ffmpeg_command(
        &["-y", "-i", &src_str, "-ss", &s_str, "-to", &e_str, "-c:a", "pcm_s16le", "-ac", "1", &dst_str],
        resource_dir,
    )
    .await?;
    Ok(())
}

fn estimate_word_timings(text: &str, duration_sec: f64) -> Vec<WordTiming> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let total_chars: usize = words.iter().map(|w| w.len()).sum();
    let time_per_char = if total_chars > 0 { duration_sec / (total_chars as f64) } else { 0.1 };

    let mut out = Vec::new();
    let mut t = 0.0;

    for w in words {
        let len = (w.len() as f64) * time_per_char;
        out.push(WordTiming {
            text: w.to_string(),
            start: t,
            end: t + len,
        });
        t += len;
    }
    out
}

async fn whisper_times(
    execution: Option<&CaptionExecution>,
    take: &Path,
    words: &[String],
    lang: &str,
    total: f64,
    tmp: &Path,
    token: &tokio_util::sync::CancellationToken,
    resource_dir: Option<&Path>,
) -> Result<Option<Vec<WordTiming>>, String> {
    let Some(execution) = execution else { return Ok(None); };
    if !execution.available(resource_dir) || words.is_empty() {
        return Ok(None);
    }
    time_script(execution, resource_dir, take, words, lang, total, tmp, token).await.map(Some)
}

async fn speko_take(key: &str, voice: &str, model: &str, lang: &str, group: &[ClipRow], tmp: &Path, ctx: &TaskCtx) -> Result<Vec<Segment>, String> {
    let clip = group.first().ok_or("Klip Speko kosong")?;
    let (wav, words) = crate::services::speko::speak(key,model,voice,&strip_voice_tags(&clip.narration),lang,&ctx.token).await?;
    let duration_ms = ((wav.len() - 44) as f64 / 48.0).round() as i64;
    let file = tmp.join(format!("speko-{}.wav",Uuid::new_v4()));
    fs::write(&file,wav).map_err(|e| e.to_string())?;
    Ok(vec![Segment {clip:clip.clone(),file,duration_ms,words,estimated:false}])
}

async fn gemini_take(
    key: &str,
    model: &str,
    voice: &str,
    lang: &str,
    group: &[ClipRow],
    tmp: &Path,
    ctx: &TaskCtx,
    whisper: Option<&CaptionExecution>,
    resource_dir: Option<&Path>,
    base: f64,
    span: f64,
) -> Result<Vec<Segment>, String> {
    let joined = group
        .iter()
        .map(|c| c.narration.trim())
        .collect::<Vec<_>>()
        .join("\n<long pause>\n");

    // One request turns the whole group into audio, and it is the longest wait
    // in the job, so say so instead of leaving the bar parked at its start.
    ctx.progress(base + span * 0.05, Some("Meminta suara ke Gemini"));
    let wav_bytes = gemini::speak_with_model(key, model, &joined, voice, lang).await?;
    let take_file = tmp.join(format!("take-{}.wav", Uuid::new_v4()));
    fs::write(&take_file, &wav_bytes).map_err(|e| e.to_string())?;

    split_take(&take_file, group, lang, tmp, ctx, whisper, resource_dir, base, span, false).await
}

async fn split_take(take_file: &Path, group: &[ClipRow], lang: &str, tmp: &Path, ctx: &TaskCtx, whisper: Option<&CaptionExecution>, resource_dir: Option<&Path>, base: f64, span: f64, strict: bool) -> Result<Vec<Segment>, String> {
    ctx.progress(base + span * 0.55, Some("Mengukur suara"));
    let total_ms = probe_duration_ms(&take_file).await.unwrap_or(2000.0);
    let total = total_ms / 1000.0;
    let silences = detect_silences(&take_file, resource_dir).await.unwrap_or_default();

    let per_scene: Vec<Vec<String>> = group.iter().map(|c| spoken_words(&c.narration)).collect();
    let all_words: Vec<String> = per_scene.concat();

    if let Some(execution) = whisper.filter(|w| w.available(resource_dir) && !all_words.is_empty()) {
        ctx.progress(base + span * 0.62, Some(execution.message()));
    }
    let timed = match whisper_times(whisper, &take_file, &all_words, lang, total, tmp, &ctx.token, resource_dir).await {
        Ok(timed) => timed,
        Err(error) => {
            if strict { return Err(error); }
            // Keep the paid TTS recording; estimated captions can be synchronized again.
            ctx.progress(base + span * 0.68, Some(&format!("Sinkronisasi gagal; caption sementara memakai perkiraan: {error}")));
            None
        }
    };
    if ctx.token.is_cancelled() { return Err("Dibatalkan".into()); }

    let mut ranges: Vec<Vec<WordTiming>> = Vec::new();
    let mut known: Option<Vec<f64>> = None;

    if let Some(ref t_words) = timed {
        let mut at = 0;
        for w in &per_scene {
            let slice = if at + w.len() <= t_words.len() {
                t_words[at..at + w.len()].to_vec()
            } else {
                Vec::new()
            };
            at += w.len();
            ranges.push(slice);
        }

        let mut k_cuts = Vec::new();
        let mut valid = true;
        for k in 0..group.len().saturating_sub(1) {
            let a = ranges.get(k).and_then(|r| r.last().map(|w| w.end));
            let b = ranges.get(k + 1).and_then(|r| r.first().map(|w| w.start));
            if let (Some(a_val), Some(b_val)) = (a, b) {
                k_cuts.push((a_val + b_val) / 2.0);
            } else {
                valid = false;
                break;
            }
        }
        if valid {
            known = Some(k_cuts);
        }
    }

    let lengths: Vec<f64> = group.iter().map(|c| strip_voice_tags(&c.narration).len() as f64).collect();
    let cuts = choose_cuts(total, &lengths, &silences, known.as_deref());

    let mut bounds = vec![0.0];
    bounds.extend(cuts);
    bounds.push(total);

    let mut out = Vec::new();
    for k in 0..group.len() {
        ctx.progress(
            base + span * (0.8 + 0.2 * (k as f64 / group.len() as f64)),
            Some(&format!("Memotong audio adegan {} dari {}", k + 1, group.len())),
        );
        let start = bounds[k];
        let end = bounds[k + 1];
        let len = (end - start).max(0.2);
        let seg_file = tmp.join(format!("seg-{}.wav", Uuid::new_v4()));
        cut_audio(&take_file, start, end, &seg_file, resource_dir).await?;

        let accurate = timed.is_some() && ranges.get(k).is_some_and(|r| r.iter().all(|w| w.end > w.start && w.start >= start - 0.05 && w.end <= end + 0.05));
        let words = if accurate {
            ranges
                .get(k)
                .map(|r| {
                    r.iter()
                        .map(|w| WordTiming {
                            text: w.text.clone(),
                            start: (w.start - start).clamp(0.0, len),
                            end: (w.end - start).clamp(0.0, len),
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            let lead = silences.iter().find(|s| s.start <= start + 0.01 && s.end > start).map(|s| s.end).unwrap_or(start);
            let tail = silences.iter().find(|s| s.start < end && s.end >= end - 0.01).map(|s| s.start).unwrap_or(end);
            estimate_word_timings(&strip_voice_tags(&group[k].narration), (tail - lead).max(0.3))
                .into_iter()
                .map(|w| WordTiming {
                    text: w.text,
                    start: w.start + (lead - start),
                    end: w.end + (lead - start),
                })
                .collect()
        };

        out.push(Segment {
            clip: group[k].clone(),
            file: seg_file,
            duration_ms: (len * 1000.0).round() as i64,
            words,
            estimated: !accurate,
        });
    }

    Ok(out)
}

async fn eleven_take(
    key: &str,
    voice: &str,
    model: &str,
    lang: &str,
    group: &[ClipRow],
    tmp: &Path,
    resource_dir: Option<&Path>,
) -> Result<Vec<Segment>, String> {
    let joined = group.iter().map(|c| c.narration.trim()).collect::<Vec<_>>().join("\n\n");
    let (audio_bytes, words) = elevenlabs::speak(key, voice, model, &joined, lang).await?;

    let take_file = tmp.join(format!("take-{}.mp3", Uuid::new_v4()));
    fs::write(&take_file, &audio_bytes).map_err(|e| e.to_string())?;

    let total_ms = probe_duration_ms(&take_file).await.unwrap_or(2000.0);
    let total = total_ms / 1000.0;

    let counts: Vec<usize> = group.iter().map(|c| spoken_words(&c.narration).len()).collect();
    let expected: usize = counts.iter().sum();
    let scale = if expected > 0 { (words.len() as f64) / (expected as f64) } else { 1.0 };

    let mut ranges = Vec::new();
    let mut at = 0usize;
    for n in counts {
        let from = ((at as f64) * scale).round() as usize;
        at += n;
        let to = (((at as f64) * scale).round() as usize).max(from);
        let slice = if from < words.len() {
            words[from..to.min(words.len())].to_vec()
        } else {
            Vec::new()
        };
        ranges.push(slice);
    }

    let mut bounds = vec![0.0];
    for k in 0..group.len().saturating_sub(1) {
        let a = ranges.get(k).and_then(|r| r.last().map(|w| w.end));
        let b = ranges.get(k + 1).and_then(|r| r.first().map(|w| w.start));
        if let (Some(a_val), Some(b_val)) = (a, b) {
            bounds.push((a_val + b_val) / 2.0);
        } else {
            bounds.push((total * ((k + 1) as f64)) / (group.len() as f64));
        }
    }
    bounds.push(total);

    let mut out = Vec::new();
    for k in 0..group.len() {
        let start = bounds[k];
        let end = (bounds[k + 1]).max(start + 0.3);
        let len = end - start;
        let seg_file = tmp.join(format!("seg-{}.wav", Uuid::new_v4()));
        cut_audio(&take_file, start, end, &seg_file, resource_dir).await?;

        let r_words = ranges.get(k).cloned().unwrap_or_default();
        let adj_words = r_words
            .into_iter()
            .map(|w| WordTiming {
                text: w.text,
                start: (w.start - start).max(0.0),
                end: (w.end - start).max(0.0),
            })
            .collect();

        out.push(Segment {
            clip: group[k].clone(),
            file: seg_file,
            duration_ms: (len * 1000.0).round() as i64,
            words: adj_words,
            estimated: false,
        });
    }

    Ok(out)
}

fn segment_asset(project_id: &str, provider: &str, model: &str, seg: &Segment, voice: &str, take_id: &str) -> Result<NewAsset,String> {
    let rel_path = format!("audio/{}.wav", Uuid::new_v4());
    let abs_path = asset_abs_path(project_id, &rel_path);

    if let Some(parent) = abs_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::copy(&seg.file, &abs_path).map_err(|e| e.to_string())?;

    let meta = json!({
        "words": seg.words,
        "estimatedTimings": seg.estimated,
        "voice": voice,
        "take": take_id
    });

    Ok(NewAsset {
        project_id: project_id.to_string(),
        clip_id: Some(seg.clip.id.clone()),
        character_id: None,
        kind: "audio".to_string(),
        provider: provider.to_string(),
        model: Some(model.to_string()),
        prompt: Some(seg.clip.narration.clone()),
        remote_id: None,
        remote_url: None,
        local_path: rel_path,
        duration_ms: Some(seg.duration_ms),
        width: None,
        height: None,
        meta,
    })


}

fn store_segment(
    db: &DbPool,
    app: &AppHandle,
    project_id: &str,
    provider: &str,
    model: &str,
    seg: &Segment,
    voice: &str,
    take_id: &str,
) -> Result<(), String> {
    let new_asset = segment_asset(project_id, provider, model, seg, voice, take_id)?;

    let asset_row = {
        let conn = db.lock().unwrap();
        insert_asset(&conn, new_asset)?
    };

    let dur = seg.duration_ms.max(1500);
    {
        let conn = db.lock().unwrap();
        patch_clip(
            &conn,
            &seg.clip.id,
            json!({
                "audioAssetId": asset_row.id,
                "durationMs": dur
            }),
        )?;
    }

    let _ = app.emit("event:asset", &asset_row);
    let _ = app.emit(
        "event:clip",
        json!({
            "clipId": seg.clip.id,
            "patch": {
                "audioAssetId": asset_row.id,
                "durationMs": dur
            }
        }),
    );

    Ok(())
}

pub fn generate_narration(
    db: DbPool,
    job_mgr: Arc<JobManager>,
    app: AppHandle,
    project_id: String,
    mode: String,
    resource_dir: Option<PathBuf>,
) -> Result<serde_json::Value, String> {
    let project = {
        let conn = db.lock().unwrap();
        if let Some(j) = active_project_job(&conn, &project_id, "narration") {
            return Ok(j);
        }
        get_project(&conn, &project_id).ok_or("Proyek tidak ditemukan")?
    };

    let targets = {
        let conn = db.lock().unwrap();
        clips_of(&conn, &project_id)
            .into_iter()
            .filter(|c| !c.narration.trim().is_empty() && (mode == "all" || c.audio_asset_id.is_none()))
            .collect::<Vec<_>>()
    };

    if targets.is_empty() {
        return Err("Tidak ada naskah narasi yang perlu disuarakan.".into());
    }

    let clip_ids: Vec<String> = targets.iter().map(|c| c.id.clone()).collect();
    let job = {
        let conn = db.lock().unwrap();
        insert_job(
            &conn,
            &project_id,
            "narration",
            &project.tts_provider,
            json!({ "mode": mode, "clipIds": clip_ids }),
            None,
            None,
        )?
    };

    let j_id = job.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let _ = app.emit("event:job", &job);

    let db_clone = db.clone();
    let p_id = project_id.clone();
    let app_clone = app.clone();
    let res_dir = resource_dir;

    job_mgr.spawn_task(&j_id, QueueName::Tts, move |ctx| async move {
        let (provider, voice, lang, model, key, whisper) = {
            let conn = db_clone.lock().unwrap();
            let p = get_project(&conn, &p_id).ok_or("Proyek tidak ditemukan")?;
            let s = get_settings(&conn);
            let prov = p.tts_provider.clone();
            let k = get_secret(&conn, &prov).unwrap_or_default();
            let m = if prov == "gemini" {
                // Empty when unset; gemini::speak_with_model turns that into a
                // message asking the user to pick a voice model in Pengaturan.
                s.get("geminiTtsModel").and_then(|x| x.as_str()).unwrap_or("").to_string()
            } else if prov == "speko" {
                s.get("spekoTtsModel").and_then(|x| x.as_str()).unwrap_or("").to_string()
            } else {
                s.get("elevenModel").and_then(|x| x.as_str()).unwrap_or("eleven_multilingual_v2").to_string()
            };
            let v = if !p.tts_voice.is_empty() {
                p.tts_voice.clone()
            } else if prov == "gemini" {
                "Charon".to_string()
            } else {
                String::new()
            };
            let whisper = if prov == "gemini" && s["whisperAuto"].as_bool().unwrap_or(true) { Some(CaptionExecution::from_settings(&conn)?) } else { None };
            (prov, v, p.language.clone(), m, k, whisper)
        };

        if provider == "elevenlabs" && voice.is_empty() {
            return Err("Pilih suara ElevenLabs dulu di langkah Ide cerita.".into());
        }

        let clips = {
            let conn = db_clone.lock().unwrap();
            targets
                .into_iter()
                .filter_map(|t| get_clip(&conn, &t.id))
                .filter(|c| !c.narration.trim().is_empty())
                .collect::<Vec<_>>()
        };

        let groups = group_clips(&clips, &provider);
        let tmp = user_data_dir().join("tmp").join(format!("narration-{}", ctx.job_id));
        let _ = fs::create_dir_all(&tmp);
        let take_id = Uuid::new_v4().to_string();

        let mut done = 0usize;
        let mut estimated = 0usize;
        let total_clips = clips.len();

        for (g_idx, group) in groups.iter().enumerate() {
            if ctx.token.is_cancelled() {
                let _ = fs::remove_dir_all(&tmp);
                return Err("Dibatalkan".into());
            }

            // Give each group the slice of the bar its own scenes cover, so the
            // nested steps below can report progress inside that slice.
            let base = 0.05 + 0.85 * (done as f64 / total_clips as f64);
            let span = 0.85 * (group.len() as f64 / total_clips as f64);
            let msg = if groups.len() > 1 {
                format!("Membuat suara bagian {} dari {}", g_idx + 1, groups.len())
            } else {
                "Membuat suara narasi".to_string()
            };
            ctx.progress(base, Some(&msg));

            let segments_res = if provider == "speko" {
                speko_take(&key,&voice,&model,&lang,group,&tmp,&ctx).await
            } else if provider == "elevenlabs" {
                eleven_take(&key, &voice, &model, &lang, group, &tmp, res_dir.as_deref()).await
            } else {
                gemini_take(&key, &model, &voice, &lang, group, &tmp, &ctx, whisper.as_ref(), res_dir.as_deref(), base, span).await
            };

            let segments = match segments_res {
                Ok(s) => s,
                Err(e) => {
                    let _ = fs::remove_dir_all(&tmp);
                    return Err(e);
                }
            };

            for s in &segments {
                estimated += usize::from(s.estimated);
                store_segment(&db_clone, &app_clone, &p_id, &provider, &model, s, &voice, &take_id)?;
            }
            done += group.len();
        }

        let _ = fs::remove_dir_all(&tmp);
        {
            let conn = db_clone.lock().unwrap();
            let _ = touch_project(&conn, &p_id, None);
        }

        Ok(Some(if estimated > 0 {
            format!("{total_clips} adegan disuarakan; {estimated} caption masih perkiraan. Sinkronkan lagi lewat menu Caption.")
        } else { format!("{total_clips} adegan selesai disuarakan") }))
    });

    Ok(job)
}

pub fn sync_captions(
    repair: bool,
    db: DbPool,
    job_mgr: Arc<JobManager>,
    app: AppHandle,
    project_id: String,
    resource_dir: Option<PathBuf>,
) -> Result<serde_json::Value, String> {
    let _project = {
        let conn = db.lock().unwrap();
        if active_project_job(&conn, &project_id, "narration").is_some() || (repair && active_project_job(&conn, &project_id, "export").is_some()) { return Err("Tunggu narasi dan ekspor selesai sebelum memperbaiki audio".into()); }
        if let Some(j) = active_project_job(&conn, &project_id, "align") {
            return Ok(j);
        }
        get_project(&conn, &project_id).ok_or("Proyek tidak ditemukan")?
    };

    let execution = {
        let conn = db.lock().unwrap();
        CaptionExecution::from_settings(&conn)?
    };
    if !execution.available(resource_dir.as_deref()) {
        return Err("Model Whisper belum diunduh. Unduh dulu di Pengaturan, bagian Caption akurat.".into());
    }

    let targets = {
        let conn = db.lock().unwrap();
        clips_of(&conn, &project_id)
            .into_iter()
            .filter(|c| c.audio_asset_id.as_ref().and_then(|id| get_asset(&conn, id)).is_some())
            .collect::<Vec<_>>()
    };

    if targets.is_empty() {
        return Err("Belum ada suara narasi untuk disinkronkan.".into());
    }

    let job = {
        let conn = db.lock().unwrap();
        insert_job(&conn, &project_id, "align", execution.provider(), json!({"target": execution.target()}), None, None)?
    };

    let j_id = job.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let _ = app.emit("event:job", &job);

    let db_clone = db.clone();
    let p_id = project_id.clone();
    let app_clone = app.clone();

    job_mgr.spawn_task(&j_id, QueueName::Whisper, move |ctx| async move {
        let tmp = user_data_dir().join("tmp").join(format!("align-{}", ctx.job_id));
        let _ = fs::create_dir_all(&tmp);

        let targets_count = targets.len();
        let result = async {
            if repair {
                let (project, sources) = {
                    let conn = db_clone.lock().unwrap();
                    let project = get_project(&conn, &p_id).ok_or("Proyek tidak ditemukan")?;
                    let all = clips_of(&conn,&p_id);
                    if all.len() != targets.len() { return Err("Semua klip harus memiliki narasi sebelum pembagian diperbaiki".to_string()); }
                    let mut sources = Vec::new();
                    for clip in &targets {
                        let audio = clip.audio_asset_id.as_ref().and_then(|id| get_asset(&conn,id)).ok_or("Suara klip tidak ditemukan")?;
                        if audio.provider.as_deref() != Some("gemini") || audio.prompt.as_deref().unwrap_or("").trim() != clip.narration.trim() { return Err("Perbaikan pembagian memerlukan rekaman Gemini utuh dengan naskah yang belum berubah. Rekam ulang narasi untuk naskah baru.".to_string()); }
                        sources.push(audio);
                    }
                    let take = sources[0].meta["take"].as_str().ok_or("Rekaman lama tidak menyimpan identitas take; rekam ulang narasi")?;
                    if sources.iter().any(|a| a.meta["take"].as_str() != Some(take)) { return Err("Klip berasal dari rekaman berbeda. Rekam ulang narasi untuk menyatukannya.".into()); }
                    (project,sources)
                };
                let mut list = String::new();
                for (i,audio) in sources.iter().enumerate() {
                    let name = format!("piece-{i}.wav");
                    fs::copy(asset_abs_path(&p_id,&audio.local_path),tmp.join(&name)).map_err(|e| e.to_string())?;
                    list.push_str(&format!("file '{name}'\n"));
                }
                let list_path = tmp.join("pieces.txt"); fs::write(&list_path,list).map_err(|e| e.to_string())?;
                let take_path = tmp.join("repaired-take.wav");
                run_ffmpeg_command(&["-y","-f","concat","-safe","1","-i",&list_path.to_string_lossy(),"-ac","1","-ar","24000","-c:a","pcm_s16le",&take_path.to_string_lossy()],resource_dir.as_deref()).await?;
                let segments = split_take(&take_path,&targets,&project.language,&tmp,&ctx,Some(&execution),resource_dir.as_deref(),0.05,0.85,true).await?;
                if ctx.token.is_cancelled() { return Err("Dibatalkan".into()); }
                {
                    let conn = db_clone.lock().unwrap();
                    for (clip,audio) in targets.iter().zip(&sources) {
                        let fresh = get_clip(&conn,&clip.id).ok_or("Klip dihapus selama perbaikan")?;
                        if fresh.audio_asset_id.as_deref() != Some(&audio.id) || fresh.narration != clip.narration { return Err("Narasi berubah selama perbaikan; hasil tidak diterapkan".into()); }
                    }
                }
                let take_id = Uuid::new_v4().to_string();
                let prepared = segments.iter().map(|seg| segment_asset(&p_id,"gemini",sources[0].model.as_deref().unwrap_or(""),seg,sources[0].meta["voice"].as_str().unwrap_or(""),&take_id)).collect::<Result<Vec<_>,_>>()?;
                if ctx.token.is_cancelled() { return Err("Dibatalkan".into()); }
                let fresh_assets = {
                    let conn = db_clone.lock().unwrap();
                    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
                    let mut fresh = Vec::new();
                    for (seg,asset) in segments.iter().zip(prepared) {
                        let current = get_clip(&tx,&seg.clip.id).ok_or("Klip dihapus selama perbaikan")?;
                        if current.audio_asset_id != seg.clip.audio_asset_id || current.narration != seg.clip.narration { return Err("Klip berubah selama perbaikan; hasil tidak diterapkan".into()); }
                        let row = insert_asset(&tx,asset)?;
                        patch_clip(&tx,&seg.clip.id,json!({"audioAssetId":row.id,"durationMs":seg.duration_ms.max(1500)}))?;
                        fresh.push(row);
                    }
                    tx.commit().map_err(|e| e.to_string())?;
                    fresh
                };
                for (seg,row) in segments.iter().zip(fresh_assets) {
                    let _ = app_clone.emit("event:asset", &row);
                    let _ = app_clone.emit("event:clip",json!({"clipId":seg.clip.id,"patch":{"audioAssetId":row.id,"durationMs":seg.duration_ms.max(1500)}}));
                }
                return Ok(Some("Pembagian klip dan caption diperbaiki dari audio lama, tanpa meminta TTS baru. Durasi total mungkin berubah sedikit.".into()));
            }
            let mut synced = 0usize;
            for (i, t) in targets.iter().enumerate() {
                if ctx.token.is_cancelled() {
                    let _ = fs::remove_dir_all(&tmp);
                    return Err("Dibatalkan".into());
                }

                ctx.progress(0.03 + 0.95 * (i as f64 / targets_count as f64), Some(&format!("{} · klip {} dari {}", execution.message(), i + 1, targets_count)));

                let (asset_opt, lang) = {
                    let conn = db_clone.lock().unwrap();
                    let c_opt = get_clip(&conn, &t.id);
                    let a = c_opt.as_ref().and_then(|c| c.audio_asset_id.as_ref()).and_then(|aid| get_asset(&conn, aid));
                    let p = get_project(&conn, &p_id);
                    (a, p.map(|x| x.language).unwrap_or_else(|| "id".to_string()))
                };

                if let Some(asset) = asset_opt {
                    let prompt = asset.prompt.clone().unwrap_or_default();
                    let words = spoken_words(&prompt);
                    if words.is_empty() {
                        continue;
                    }
                    let seconds = (asset.duration_ms.unwrap_or(2000) as f64) / 1000.0;
                    let audio_abs = asset_abs_path(&p_id, &asset.local_path);

                    let timed = time_script(&execution, resource_dir.as_deref(), &audio_abs, &words, &lang, seconds, &tmp, &ctx.token).await
                        .map_err(|e| format!("Sinkronisasi klip {} gagal: {e}. {synced} klip sebelumnya sudah tersimpan.", i + 1))?;
                    let mut meta = asset.meta.clone();
                    if let Some(obj) = meta.as_object_mut() {
                        obj.insert("words".to_string(), json!(timed));
                        obj.insert("estimatedTimings".to_string(), json!(false));
                        obj.insert("timedBy".to_string(), json!(execution.provider()));
                    }
                    let conn = db_clone.lock().unwrap();
                    update_asset_meta(&conn, &asset.id, meta)?;
                    synced += 1;
                    if let Some(fresh) = get_asset(&conn, &asset.id) {
                        let _ = app_clone.emit("event:asset", &fresh);
                    }
                }
            }
            Ok(Some(format!("{synced} klip disinkronkan")))
        }.await;
        let _ = fs::remove_dir_all(&tmp);
        result
    });

    Ok(job)
}
