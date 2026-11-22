use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::db::repo::{get_bundle, insert_job, touch_project, AssetRow, ClipRow, ProjectBundle};
use crate::db::DbPool;
use crate::jobs::{JobManager, QueueName, TaskCtx};
use crate::media::ass::{build_ass_content, build_srt, timeline_chunks};
use crate::media::font_metrics::FontMetricsCache;
use crate::paths::{asset_abs_path, ffmpeg_path, fonts_dir, user_data_dir};
use crate::speech::align::WordTiming;
use crate::media::ssh_export::SshExport;

pub const TRANSITION_SEC: f64 = 0.4;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    pub fps: u32,
    pub resolution: u32, // 720, 1080, 1440, 2160
    pub burn_captions: bool,
    pub write_srt: bool,
    pub duck_music: bool,
    pub normalize_audio: bool,
    pub folder: String,
    pub file_name: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub ssh_profile: String,
    #[serde(default)]
    pub ssh_profiles: Vec<String>,
    #[serde(default)]
    pub delete_after_download: bool,
}

#[derive(Debug, Clone)]
pub struct OutputSize {
    pub w: u32,
    pub h: u32,
}

pub fn output_size(aspect: &str, res: u32) -> OutputSize {
    let long = match res {
        720 => 1280,
        1080 => 1920,
        1440 => 2560,
        2160 => 3840,
        _ => 1920,
    };
    let short = res;
    if aspect == "16:9" {
        OutputSize { w: long, h: short }
    } else {
        OutputSize { w: short, h: long }
    }
}

pub fn strength_amount(strength: &str) -> f64 {
    match strength {
        "halus" => 0.08,
        "sedang" => 0.15,
        "kuat" => 0.25,
        _ => 0.08,
    }
}

pub struct CameraZoompan {
    pub z: String,
    pub x: String,
    pub y: String,
}

pub fn camera_zoompan(preset: &str, strength: &str, frames: u32, fps: u32) -> CameraZoompan {
    let s = strength_amount(strength);
    let n = (frames.max(2) - 1).max(1);
    let e = format!("(min(on/{n},1)*min(on/{n},1)*(3-2*min(on/{n},1)))");
    let t = format!("(on/{fps})");
    let lin = |a: f64, b: f64| -> String { format!("({a}+({b_diff})*{e})", b_diff = b - a) };

    let mut z = "1".to_string();
    let mut x = "0".to_string();
    let mut y = "0".to_string();

    match preset {
        "zoomin" => {
            z = lin(1.0, 1.0 + s);
        }
        "zoomout" => {
            z = lin(1.0 + s, 1.0);
        }
        "panleft" => {
            z = format!("{}", 1.0 + s);
            x = lin(0.9, -0.9);
        }
        "panright" => {
            z = format!("{}", 1.0 + s);
            x = lin(-0.9, 0.9);
        }
        "kenburns" => {
            z = lin(1.0 + s * 0.3, 1.0 + s);
            x = lin(-0.5, 0.5);
            y = lin(0.3, -0.3);
        }
        "shake" => {
            z = format!("{}", 1.0 + (s * 0.5).max(0.04));
            x = format!("(0.35*sin(2*PI*1.3*{t}))");
            y = format!("(0.3*sin(2*PI*1.7*{t}+1))");
        }
        _ => {}
    }

    CameraZoompan {
        z,
        x: format!("(iw-iw/zoom)*(1+{x})/2"),
        y: format!("(ih-ih/zoom)*(1+{y})/2"),
    }
}

pub struct AnchorOffset {
    pub ax: f64,
    pub ay: f64,
}

pub fn anchor_offset(anchor: u32) -> AnchorOffset {
    let col = (anchor.saturating_sub(1)) % 3;
    let row = (anchor.saturating_sub(1)) / 3;
    let ax = (col as f64) / 2.0;
    let ay = if row == 2 {
        0.0
    } else if row == 1 {
        0.5
    } else {
        1.0
    };
    AnchorOffset { ax, ay }
}

fn parse_time_sec(s: &str) -> Option<f64> {
    if let Some(pos) = s.find("time=") {
        let rem = &s[pos + 5..];
        let token = rem.split_whitespace().next().unwrap_or("");
        let parts: Vec<&str> = token.split(':').collect();
        if parts.len() == 3 {
            let h: f64 = parts[0].parse().ok()?;
            let m: f64 = parts[1].parse().ok()?;
            let sec: f64 = parts[2].parse().ok()?;
            return Some(h * 3600.0 + m * 60.0 + sec);
        }
    }
    None
}

async fn run_ffmpeg<F>(
    args: &[String],
    cwd: &Path,
    token: &CancellationToken,
    total_sec: Option<f64>,
    mut on_progress: F,
    resource_dir: Option<&Path>,
    remote: &mut Option<SshExport>,
) -> Result<(), String>
where
    F: FnMut(f64),
{
    if total_sec.is_none() {
        if let Some(session) = remote.as_mut() {
            let option = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|s| s.parse::<f64>().ok());
            let duration = option("-t").or_else(|| Some(option("-frames:v")? / option("-r").unwrap_or(30.0))).unwrap_or(1.0);
            session.queue(args, duration)?;
            return Ok(());
        }
    }
    if let Some(session) = remote.as_mut() {
        return session.run(args, cwd, total_sec.unwrap_or(1.0), token, on_progress).await;
    }
    let mut cmd = Command::new(ffmpeg_path(resource_dir));
    cmd.arg("-hide_banner").arg("-y")
        .args(["-filter_threads", "2", "-filter_complex_threads", "2"])
        .args(args).current_dir(cwd);
    cmd.kill_on_drop(true).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);

    let mut child = cmd.spawn().map_err(|e| format!("Gagal menjalankan proses render: {e}"))?;

    let stderr = child.stderr.take().ok_or("Failed to capture FFmpeg stderr")?;
    let mut reader = BufReader::new(stderr).lines();

    let mut tail = Vec::new();

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                let _ = child.kill().await;
                if let Some(session) = remote.as_ref() { session.cancel().await; }
                return Err("Dibatalkan".into());
            }
            line_res = reader.next_line() => {
                match line_res {
                    Ok(Some(line)) => {
                        if tail.len() > 20 {
                            tail.remove(0);
                        }
                        tail.push(line.clone());

                        if let Some(tot) = total_sec {
                            if tot > 0.0 {
                                if let Some(sec) = parse_time_sec(&line) {
                                    let prog = (sec / tot).clamp(0.0, 1.0);
                                    on_progress(prog);
                                }
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
        }
    }

    let status = child.wait().await.map_err(|e| e.to_string())?;
    if token.is_cancelled() {
        return Err("Dibatalkan".into());
    }
    if status.success() {
        Ok(())
    } else {
        let msg = tail.join("\n");
        Err(format!("FFmpeg gagal (exit code {:?}): {msg}", status.code()))
    }
}

#[derive(Clone)]
pub struct SegmentPlan {
    pub clip: ClipRow,
    pub start: f64,
    pub own: f64,
    pub overlap: f64,
    pub transition: String,
}

pub fn export_profiles(opts: &ExportOptions) -> Result<Vec<String>, String> {
    let mut profiles = Vec::new();
    for profile in std::iter::once(&opts.ssh_profile).chain(opts.ssh_profiles.iter()) {
        crate::media::ssh_export::validate_profile(profile)?;
        if !profiles.iter().any(|p: &String| p.eq_ignore_ascii_case(profile)) { profiles.push(profile.clone()); }
    }
    Ok(profiles)
}

fn distribute_segments(segs: &[SegmentPlan], nodes: usize) -> Vec<Vec<usize>> {
    let count = nodes.min(segs.len());
    let mut groups = Vec::new();
    let mut cursor = 0;
    let mut remaining: f64 = segs.iter().map(|s|s.own + s.overlap).sum();
    for node in 0..count {
        let target = remaining / (count - node) as f64;
        let start = cursor;
        let mut duration = 0.0;
        while cursor < segs.len() - (count - node - 1) {
            let next = segs[cursor].own + segs[cursor].overlap;
            if cursor > start && node + 1 < count && (duration + next - target).abs() > (duration - target).abs() { break; }
            duration += next;
            cursor += 1;
        }
        groups.push((start..cursor).collect());
        remaining -= duration;
    }
    groups
}

fn merged_plan(segs: &[SegmentPlan]) -> SegmentPlan {
    let mut plan = segs[0].clone();
    plan.own = segs.iter().map(|s|s.own).sum();
    plan.overlap = segs.last().unwrap().overlap;
    plan.transition = segs.last().unwrap().transition.clone();
    plan
}

async fn merge_video_parts(files: Vec<String>, segs: Vec<SegmentPlan>, group: usize, opts: &ExportOptions,
    tmp: &Path, token: &CancellationToken, resource_dir: Option<&Path>, remote: &mut Option<SshExport>) -> Result<(String, SegmentPlan),String> {
    // Bound decoder/frame buffers instead of opening the entire movie in one filter graph.
    let plan = merged_plan(&segs);
    let mut parts: Vec<(String,SegmentPlan)> = files.into_iter().zip(segs).collect();
    let mut level = 0;
    while parts.len() > 1 {
        let mut next = Vec::new();
        for (batch, chunk) in parts.chunks(4).enumerate() {
            if chunk.len() == 1 { next.push(chunk[0].clone()); continue; }
            let mut plans: Vec<SegmentPlan> = chunk.iter().map(|(_,s)|s.clone()).collect();
            let merged = merged_plan(&plans);
            for p in &mut plans { p.start -= merged.start; }
            let (graph, output) = video_graph(&plans,opts.fps);
            let graph_path = tmp.join(format!("merge_{group}_{level}_{batch}.txt"));
            fs::write(&graph_path,graph.join(";\n")).map_err(|e|e.to_string())?;
            let name = if parts.len() <= 4 { format!("seg_{:06}.mp4",100000+group) } else { format!("merge_{group}_{level}_{batch}.mp4") };
            let mut args = Vec::new();
            for (file,_) in chunk { args.extend(["-threads".into(),"1".into(),"-i".into(),file.clone()]); }
            args.extend(["-filter_complex_script".into(),graph_path.to_string_lossy().to_string(),"-map".into(),output,
                "-an".into(),"-c:v".into(),"libx264".into(),"-threads".into(),"2".into(),"-preset".into(),"veryfast".into(),
                "-crf".into(),"17".into(),"-pix_fmt".into(),"yuv420p".into(),"-r".into(),opts.fps.to_string(),
                "-t".into(),format!("{:.6}",merged.own + merged.overlap),name.clone()]);
            run_ffmpeg(&args,tmp,token,None,|_|{},resource_dir,remote).await?;
            next.push((name,merged));
        }
        parts = next;
        level += 1;
    }
    Ok((parts.pop().ok_or("Tidak ada klip untuk digabung")?.0,plan))
}

pub fn plan_segments(clips: &[ClipRow], fps: u32) -> Vec<SegmentPlan> {
    let fps = fps.max(1) as f64;
    let mut elapsed = 0.0;
    let mut boundary = 0u64;
    let mut segs = Vec::new();
    let n = clips.len();

    for (i, clip) in clips.iter().enumerate() {
        // Round cumulative boundaries, not individual lengths, to avoid drift.
        elapsed += (clip.duration_ms.max(1) as f64) / 1000.0;
        let end = ((elapsed * fps).round() as u64).max(boundary + 1);
        let start = boundary as f64 / fps;
        let own = (end - boundary) as f64 / fps;
        let last = i == n - 1;
        let tr_str = match clip.transition.as_str() {
            "slideleft" => "slideleft",
            "zoomin" => "zoomin",
            "cut" => "",
            _ => "fade",
        };
        let overlap = if last || tr_str.is_empty() {
            0.0
        } else {
            (TRANSITION_SEC * fps).round().max(1.0) / fps
        };

        segs.push(SegmentPlan {
            clip: clip.clone(),
            start,
            own,
            overlap,
            transition: if tr_str.is_empty() { "fade".to_string() } else { tr_str.to_string() },
        });
        boundary = end;
    }

    segs
}

fn video_graph(segs: &[SegmentPlan], fps: u32) -> (Vec<String>, String) {
    if segs.len() == 1 {
        return (Vec::new(), "[0:v]".to_string());
    }

    let mut graph = Vec::new();
    let mut blocks = Vec::new();
    let mut start = 0;

    // Concatenate runs of hard cuts before chaining fades. A concat output has
    // a different time base from MP4 inputs, so normalize every block for xfade.
    for i in 0..segs.len() {
        if segs[i].overlap > 0.0 || i == segs.len() - 1 {
            let label = format!("[block{}]", blocks.len());
            if start == i {
                graph.push(format!("[{start}:v]settb=AVTB{label}"));
            } else {
                let inputs = (start..=i).map(|n| format!("[{n}:v]")).collect::<String>();
                graph.push(format!(
                    "{inputs}concat=n={}:v=1:a=0,settb=AVTB{label}",
                    i - start + 1
                ));
            }
            blocks.push((i, label));
            start = i + 1;
        }
    }

    let mut v = blocks[0].1.clone();
    for (i, (last, _)) in blocks.iter().take(blocks.len() - 1).enumerate() {
        let seg = &segs[*last];
        let next = &blocks[i + 1].1;
        let label = format!("[x{i}]");
        let offset = segs[*last + 1].start;
        graph.push(format!(
            "{v}{next}xfade=transition={tr}:duration={dur:.6}:offset={off:.6}{label}",
            tr = seg.transition,
            dur = seg.overlap,
            off = offset
        ));
        v = label;
    }

    // Keep graph timestamps frame-aligned for output and overlays.
    graph.push(format!("{v}fps={fps},settb=AVTB[vbase]"));
    (graph, "[vbase]".to_string())
}

fn find_asset<'a>(bundle: &'a ProjectBundle, id: Option<&str>) -> Option<&'a AssetRow> {
    let aid = id?;
    bundle.assets.iter().find(|a| a.id == aid)
}

fn narration_graph(segs: &[SegmentPlan], inputs: &[Option<usize>]) -> Vec<String> {
    assert_eq!(segs.len(), inputs.len());
    if inputs.iter().all(Option::is_none) { return Vec::new(); }
    // Delaying every voice to its absolute start allocates silence buffers for the whole timeline.
    let mut graph: Vec<String> = segs.iter().zip(inputs).enumerate().map(|(i, (seg, input))| {
        let source = match input {
            Some(index) => format!("[{index}:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,apad"),
            None => "anullsrc=r=48000:cl=stereo".into(),
        };
        format!("{source},atrim=duration={:.6},asetpts=PTS-STARTPTS[a{i}]", seg.own)
    }).collect();
    let voices = (0..segs.len()).map(|i|format!("[a{i}]")).collect::<String>();
    graph.push(format!("{voices}concat=n={}:v=0:a=1[voice]", segs.len()));
    graph
}

fn image_overlay_filters(o: &Value, size: &OutputSize, total: f64, input: usize, k: usize, video: &str) -> [String; 2] {
    let width = o.get("width").and_then(Value::as_f64).unwrap_or(0.14);
    let opacity = o.get("opacity").and_then(Value::as_f64).unwrap_or(0.9).clamp(0.0, 1.0);
    let anchor = o.get("anchor").and_then(Value::as_u64).unwrap_or(9) as u32;
    let x = o.get("x").and_then(Value::as_f64).unwrap_or(0.5) * size.w as f64;
    let y = o.get("y").and_then(Value::as_f64).unwrap_or(0.5) * size.h as f64;
    let start = o.get("start").and_then(Value::as_f64).unwrap_or(0.0).max(0.0);
    let end = o.get("end").and_then(Value::as_f64).unwrap_or(total).min(total);
    let iw = ((((width * size.w as f64) / 2.0).round() * 2.0) as u32).max(2);
    let offset = anchor_offset(anchor);
    [
        format!("[{input}:v]format=rgba,scale={iw}:-1,setsar=1,colorchannelmixer=aa={opacity:.4}[ov{k}]"),
        format!("{video}[ov{k}]overlay=x='{x:.3}-overlay_w*{ax}':y='{y:.3}-overlay_h*{ay}':enable='gte(t,{start:.3})*lt(t,{end:.3})'[vo{k}]", ax = offset.ax, ay = offset.ay),
    ]
}

fn still_image_filter(size: &OutputSize, resolution: u32, fps: u32, preset: &str, strength: &str, frames: u32) -> String {
    let ss: f64 = if resolution == 2160 { 1.5 } else { 2.0 };
    let bw = (((size.w as f64 * ss) / 2.0).round() * 2.0) as u32;
    let bh = (((size.h as f64 * ss) / 2.0).round() * 2.0) as u32;
    let zp = camera_zoompan(preset, strength, frames, fps);
    format!(
        "scale={bw}:{bh}:force_original_aspect_ratio=increase:flags=lanczos,crop={bw}:{bh},format=yuv444p,zoompan=z='{z}':x='{x}':y='{y}':d=1:s={w}x{h}:fps={fps},setsar=1,format=yuv420p",
        z = zp.z, x = zp.x, y = zp.y, w = size.w, h = size.h,
    )
}

async fn render_segment(
    bundle: &ProjectBundle,
    seg: &SegmentPlan,
    index: usize,
    opts: &ExportOptions,
    size: &OutputSize,
    tmp: &Path,
    token: &CancellationToken,
    resource_dir: Option<&Path>,
    remote: &mut Option<SshExport>,
) -> Result<String, String> {
    let clip = &seg.clip;
    let out_file = format!("seg_{:03}.mp4", index);
    let len = seg.own + seg.overlap;
    let frames = ((len * (opts.fps as f64)).round() as u32).max(1);

    let enc_args = vec![
        "-an".to_string(),
        "-c:v".to_string(),
        "libx264".to_string(),
        "-preset".to_string(),
        "veryfast".to_string(),
        "-crf".to_string(),
        "17".to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-r".to_string(),
        opts.fps.to_string(),
    ];

    if clip.motion_type == "video" {
        if let Some(video) = find_asset(bundle, clip.video_asset_id.as_deref()) {
            let in_path = asset_abs_path(&bundle.project.id, &video.local_path);
            let vf = format!(
                "scale={}:{}:force_original_aspect_ratio=increase:flags=lanczos,crop={}:{},setsar=1,fps={},tpad=stop_mode=clone:stop_duration={:.3},format=yuv420p",
                size.w, size.h, size.w, size.h, opts.fps, len
            );
            let mut args = vec![
                "-i".to_string(),
                in_path.to_string_lossy().to_string(),
                "-vf".to_string(),
                vf,
                "-frames:v".to_string(),
                frames.to_string(),
            ];
            args.extend(enc_args);
            args.push(out_file.clone());

            run_ffmpeg(&args, tmp, token, None, |_| {}, resource_dir, remote).await?;
            return Ok(out_file);
        }
    }

    let image = find_asset(bundle, clip.image_asset_id.as_deref())
        .ok_or_else(|| format!("Klip {:02} (“{}”) belum punya gambar.", index + 1, clip.title))?;
    let in_path = asset_abs_path(&bundle.project.id, &image.local_path);

    let own_frames = ((seg.own * (opts.fps as f64)).round() as u32).max(2);
    let vf = still_image_filter(size, opts.resolution, opts.fps, &clip.camera_preset, &clip.motion_strength, own_frames);

    let mut args = vec![
        "-loop".to_string(),
        "1".to_string(),
        "-framerate".to_string(),
        opts.fps.to_string(),
        "-t".to_string(),
        format!("{:.3}", len),
        "-i".to_string(),
        in_path.to_string_lossy().to_string(),
        "-vf".to_string(),
        vf,
        "-frames:v".to_string(),
        frames.to_string(),
    ];
    args.extend(enc_args);
    args.push(out_file.clone());

    run_ffmpeg(&args, tmp, token, None, |_| {}, resource_dir, remote).await?;
    Ok(out_file)
}

pub async fn render_project(
    ctx: &TaskCtx,
    project_id: &str,
    opts: &ExportOptions,
    db: &DbPool,
    resource_dir: Option<&Path>,
) -> Result<String, String> {
    let bundle = {
        let conn = db.lock().unwrap();
        get_bundle(&conn, project_id).ok_or("Proyek tidak ditemukan")?
    };

    if bundle.clips.is_empty() {
        return Err("Proyek belum punya klip".into());
    }

    let size = output_size(&bundle.project.aspect_ratio, opts.resolution);
    let tmp = user_data_dir().join("tmp").join(format!("export-{}", ctx.job_id));
    fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;

    let mut remote = if opts.target == "ssh" {
        ctx.progress(0.02, Some("Menghubungkan VPS dan menyiapkan unggahan"));
        Some(SshExport::new(&opts.ssh_profile, &ctx.job_id, &tmp, &bundle.project.title, opts.delete_after_download, &ctx.token).await?)
    } else { None };
    let result = render_project_inner(ctx, &bundle, opts, &size, &tmp, resource_dir, &mut remote).await;
    if result.is_err() && ctx.token.is_cancelled() { if let Some(session) = remote { session.cancel().await; } }

    let _ = fs::remove_dir_all(&tmp);

    if result.is_ok() {
        let conn = db.lock().unwrap();
        let _ = touch_project(&conn, project_id, Some("exported"));
    }

    result
}

async fn render_distributed_segments(
    ctx: &TaskCtx, bundle: &ProjectBundle, segs: &[SegmentPlan], opts: &ExportOptions,
    size: &OutputSize, tmp: &Path, resource_dir: Option<&Path>, primary: &SshExport,
) -> Result<(Vec<String>, Vec<SegmentPlan>), String> {
    use futures_util::StreamExt;
    let profiles = export_profiles(opts)?;
    let groups = distribute_segments(segs, profiles.len());
    let mut nodes = Vec::new();
    let mut files = Vec::new();
    let mut video_plans = Vec::new();
    for (node, indices) in groups.iter().enumerate() {
        ctx.progress(0.03, Some(&format!("Menyiapkan VPS {} · {} klip", profiles[node], indices.len())));
        let session = SshExport::new(&profiles[node], &uuid::Uuid::new_v4().to_string(), tmp,
            &format!("{} · bagian {} dari {}", bundle.project.title, node + 1, groups.len()), false, &ctx.token).await?;
        session.require_direct(primary,&ctx.token).await?;
        let mut remote = Some(session);
        let mut outputs = Vec::new();
        for &index in indices {
            let name = render_segment(bundle, &segs[index], index, opts, size, tmp, &ctx.token, resource_dir, &mut remote).await?;
            outputs.push(name);
        }
        let plans = indices.iter().map(|&i|segs[i].clone()).collect();
        let (file,plan) = merge_video_parts(outputs,plans,node,opts,tmp,&ctx.token,resource_dir,&mut remote).await?;
        files.push(file.clone());
        video_plans.push(plan);
        let outputs = vec![file];
        nodes.push((profiles[node].clone(), remote.unwrap(), outputs));
    }
    primary.prepare_receiver(&ctx.token).await?;
    ctx.set_remote(&serde_json::json!({"primary":opts.ssh_profile,"nodes":nodes.iter().map(|(profile,session,_)|serde_json::json!({"profile":profile,"id":session.id()})).collect::<Vec<_>>()} ).to_string());
    let weights: Vec<f64> = groups.iter().map(|indices|indices.iter().map(|&i|segs[i].own + segs[i].overlap).sum()).collect();
    let total: f64 = weights.iter().sum();
    let progress = Arc::new(std::sync::Mutex::new(vec![0.0; nodes.len()]));
    let cancel = ctx.token.child_token();
    let mut pending = futures_util::stream::FuturesUnordered::new();
    for (node, (profile, session, outputs)) in nodes.iter().enumerate() {
        let state = progress.clone();
        let weights = &weights;
        let token = &cancel;
        pending.push(async move {
            session.run_segments(outputs, tmp, primary, token, |fraction| {
                let combined = {let mut values=state.lock().unwrap(); values[node]=fraction;
                    values.iter().zip(weights).map(|(value,weight)|value * weight).sum::<f64>() / total.max(0.001)};
                ctx.progress(0.08 + 0.56 * combined, Some(&format!("{} · {profile}", if fraction >= 1.0 { "Memindahkan bagian langsung ke VPS utama" } else { "Merender dan menggabungkan bagian di VPS" })));
            }).await.map_err(|e| format!("VPS {profile}: {e}"))
        });
    }
    let mut failed = None;
    while let Some(result) = pending.next().await {
        if let Err(error) = result { if failed.is_none() { failed=Some(error); cancel.cancel(); } }
    }
    drop(pending);
    if let Some(error) = failed {
        futures_util::future::join_all(nodes.iter().map(|(_,session,_)|session.cancel())).await;
        return Err(error);
    }
    ctx.progress(0.65, Some("Semua klip selesai dan diverifikasi; menyiapkan penggabungan di VPS utama"));
    Ok((files,video_plans))
}

async fn render_project_inner(
    ctx: &TaskCtx,
    bundle: &ProjectBundle,
    opts: &ExportOptions,
    size: &OutputSize,
    tmp: &Path,
    resource_dir: Option<&Path>,
    remote: &mut Option<SshExport>,
) -> Result<String, String> {
    let mut clips = bundle.clips.clone();
    for clip in &mut clips {
        let audio_ms = find_asset(bundle, clip.audio_asset_id.as_deref()).and_then(|a| a.duration_ms).unwrap_or(0);
        clip.duration_ms = clip.duration_ms.max(audio_ms).max(500);
    }
    let segs = plan_segments(&clips, opts.fps);
    let total: f64 = segs.iter().map(|s| s.own).sum();

    // 1. Silent segments; shared timeline plans also preserve cross-node overlaps.
    let segs_len = segs.len();
    let mut video_segs = segs.clone();
    let mut files = vec![String::new(); segs_len];

    let mut done = 0usize;
    let distributed = remote.is_some() && segs_len > 1 && export_profiles(opts)?.len() > 1;
    if distributed {
        (files,video_segs) = render_distributed_segments(ctx, bundle, &segs, opts, size, tmp, resource_dir, remote.as_ref().unwrap()).await?;
    } else { for i in 0..segs_len {
        if ctx.token.is_cancelled() {
            return Err("Dibatalkan".into());
        }
        if remote.is_some() { ctx.progress(0.03 + 0.05 * (done as f64 / segs_len as f64), Some(&format!("Menyiapkan paket klip {} dari {} untuk VPS", i + 1, segs_len))); }
        let f = render_segment(bundle, &segs[i], i, opts, size, tmp, &ctx.token, resource_dir, remote).await?;
        files[i] = f;
        done += 1;
        let p = 0.03 + if remote.is_some() { 0.05 } else { 0.60 } * (done as f64 / segs_len as f64);
        ctx.progress(p, Some(&format!("Merender klip {} dari {}", done, segs_len)));
    }}
    if files.len() > 4 {
        ctx.progress(if distributed {0.65} else if remote.is_some() {0.07} else {0.64},Some("Menggabungkan video per kelompok kecil agar RAM tetap terkendali"));
        let (file,plan) = merge_video_parts(files,video_segs,90000,opts,tmp,&ctx.token,resource_dir,remote).await?;
        files=vec![file]; video_segs=vec![plan];
    }

    // 2. Captions & Overlays
    let f_dir = fonts_dir(resource_dir);
    let metrics = FontMetricsCache::new();

    let mut chunk_inputs: Vec<(f64, Vec<WordTiming>)> = Vec::new();
    for s in &segs {
        let words = find_asset(bundle, s.clip.audio_asset_id.as_deref()).map(|audio| {
            crate::media::ass::edited_words(&bundle.project.editor, &s.clip.id, &audio.id, &audio.meta["words"], s.own)
        }).unwrap_or_default();
        chunk_inputs.push((s.start, words));
    }

    let chunks = timeline_chunks(&chunk_inputs);
    let burn = opts.burn_captions && !chunks.is_empty();

    let overlays = bundle
        .project
        .editor
        .get("overlays")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();

    let mut text_overlays: Vec<Value> = Vec::new();
    let mut image_overlays: Vec<(Value, PathBuf)> = Vec::new();

    for o in &overlays {
        let kind = o.get("kind").and_then(|x| x.as_str()).unwrap_or("");
        let start = o.get("start").and_then(|x| x.as_f64()).unwrap_or(0.0);
        if start >= total {
            continue;
        }

        if kind == "text" {
            let t = o.get("text").and_then(|x| x.as_str()).unwrap_or("");
            if !t.trim().is_empty() {
                text_overlays.push(o.clone());
            }
        } else if kind == "image" {
            let aid = o.get("assetId").and_then(|x| x.as_str());
            if let Some(asset) = find_asset(bundle, aid) {
                let p = asset_abs_path(&bundle.project.id, &asset.local_path);
                if p.exists() {
                    image_overlays.push((o.clone(), p));
                }
            }
        }
    }

    let use_ass = burn || !text_overlays.is_empty();
    if use_ass {
        let ass_text = build_ass_content(
            if burn { &chunks } else { &[] },
            &bundle.project.editor,
            total,
            size.w,
            size.h,
            &f_dir,
            &metrics,
        );
        fs::write(tmp.join("captions.ass"), ass_text).map_err(|e| e.to_string())?;

        let tmp_fonts = tmp.join("fonts");
        fs::create_dir_all(&tmp_fonts).map_err(|e| e.to_string())?;
        if let Ok(entries) = fs::read_dir(&f_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|s| s.to_str()) == Some("ttf") {
                    if let Some(fname) = p.file_name() {
                        let _ = fs::copy(&p, tmp_fonts.join(fname));
                    }
                }
            }
        }
    }

    // 3. One ffmpeg pass: transitions + overlays + captions + audio mix
    let mut args: Vec<String> = Vec::new();
    for f in &files {
        args.extend(["-threads".to_string(), "1".to_string()]);
        args.push("-i".to_string());
        args.push(f.clone());
    }

    let (mut graph, mut v) = video_graph(&video_segs, opts.fps);

    // Logos / image overlays
    for (k, (o, p)) in image_overlays.iter().enumerate() {
        args.push("-i".to_string());
        args.push(p.to_string_lossy().to_string());

        let [scaled, composite] = image_overlay_filters(o, size, total, files.len() + k, k, &v);
        graph.push(scaled);
        graph.push(composite);
        v = format!("[vo{k}]");
    }

    let ass_filter = if use_ass { "ass=captions.ass:fontsdir=fonts," } else { "" };
    graph.push(format!("{v}{ass_filter}format=yuv420p[vout]"));

    let mut input_index = files.len() + image_overlays.len();
    let mut voices = Vec::new();

    for s in &segs {
        if let Some(audio) = find_asset(bundle, s.clip.audio_asset_id.as_deref()) {
            let p = asset_abs_path(&bundle.project.id, &audio.local_path);
            args.push("-i".to_string());
            args.push(p.to_string_lossy().to_string());

            voices.push(Some(input_index));
            input_index += 1;
        } else { voices.push(None); }
    }

    let music_aid = bundle.project.editor.get("musicAssetId").and_then(|x| x.as_str());
    let music = find_asset(bundle, music_aid);
    let mut aout: Option<String> = None;

    if voices.iter().any(Option::is_some) {
        graph.extend(narration_graph(&segs, &voices));
        aout = Some("[voice]".to_string());
    }

    if let Some(mus) = music {
        let p = asset_abs_path(&bundle.project.id, &mus.local_path);
        args.push("-stream_loop".to_string());
        args.push("-1".to_string());
        args.push("-i".to_string());
        args.push(p.to_string_lossy().to_string());

        let vol = bundle.project.editor.get("musicVolumeDb").and_then(|x| x.as_f64()).unwrap_or(-14.0);
        let fade_st = (total - 2.0).max(0.0);
        graph.push(format!(
            "[{input_index}:a]aresample=48000,aformat=channel_layouts=stereo,atrim=0:{total:.3},volume={vol}dB,afade=t=out:st={fade_st:.3}:d=2[mus]"
        ));

        if let Some(_voice_label) = aout {
            if opts.duck_music {
                graph.push("[voice]asplit=2[v1][v2]".to_string());
                graph.push("[mus][v2]sidechaincompress=threshold=0.02:ratio=8:attack=20:release=500[duck]".to_string());
                graph.push("[v1][duck]amix=inputs=2:normalize=0:dropout_transition=0[mix]".to_string());
            } else {
                graph.push("[voice][mus]amix=inputs=2:normalize=0:dropout_transition=0[mix]".to_string());
            }
            aout = Some("[mix]".to_string());
        } else {
            aout = Some("[mus]".to_string());
        }
    }

    if let Some(cur_aout) = aout {
        let loudnorm = if opts.normalize_audio { "loudnorm=I=-14:LRA=11:TP=-1.5," } else { "" };
        graph.push(format!("{cur_aout}{loudnorm}aresample=48000,apad,atrim=0:{total:.3}[aout]"));
    } else {
        graph.push(format!("anullsrc=r=48000:cl=stereo,atrim=0:{total:.3}[aout]"));
    }

    fs::write(tmp.join("graph.txt"), graph.join(";\n")).map_err(|e| e.to_string())?;

    let folder = PathBuf::from(&opts.folder);
    fs::create_dir_all(&folder).map_err(|e| e.to_string())?;

    let clean_base = regex::Regex::new(r#"[<>:"/\\|?*]+"#)
        .unwrap()
        .replace_all(&opts.file_name, "-")
        .trim_end_matches(".mp4")
        .to_string();
    let base_name = if clean_base.trim().is_empty() { "video".to_string() } else { clean_base };

    let mut out_path = folder.join(format!("{base_name}.mp4"));
    let mut n = 2;
    while out_path.exists() {
        out_path = folder.join(format!("{base_name}-{n}.mp4"));
        n += 1;
    }

    ctx.progress(if distributed { 0.66 } else if remote.is_some() { 0.08 } else { 0.64 }, Some(if distributed { "Mengirim klip selesai ke VPS utama untuk penggabungan" } else if remote.is_some() { "Mengunggah paket dan merender di VPS" } else { "Menggabungkan klip, suara, dan caption" }));

    let crf = if opts.resolution == 2160 { "20" } else { "19" };
    let mut final_args = args;
    final_args.extend(vec![
        "-filter_complex_script".to_string(),
        "graph.txt".to_string(),
        "-map".to_string(),
        "[vout]".to_string(),
        "-map".to_string(),
        "[aout]".to_string(),
        "-c:v".to_string(),
        "libx264".to_string(),
        "-threads".to_string(),
        "2".to_string(),
        "-preset".to_string(),
        "medium".to_string(),
        "-crf".to_string(),
        crf.to_string(),
        "-r".to_string(),
        opts.fps.to_string(),
        "-c:a".to_string(),
        "aac".to_string(),
        "-b:a".to_string(),
        "192k".to_string(),
        "-ar".to_string(),
        "48000".to_string(),
        "-movflags".to_string(),
        "+faststart".to_string(),
        "-t".to_string(),
        format!("{:.3}", total),
        out_path.to_string_lossy().to_string(),
    ]);

    run_ffmpeg(
        &final_args,
        tmp,
        &ctx.token,
        Some(total),
        |f| {
            ctx.progress(if distributed { 0.66 + 0.32 * f } else if opts.target == "ssh" { 0.08 + 0.90 * f } else { 0.64 + 0.34 * f }, Some(if opts.target == "ssh" && f >= 1.0 { "Mengunduh video hasil dari VPS" } else { "Menggabungkan klip, suara, dan caption" }));
        },
        resource_dir,
        remote,
    )
    .await?;

    if opts.write_srt && !chunks.is_empty() {
        let srt_path = out_path.with_extension("srt");
        let _ = fs::write(srt_path, build_srt(&chunks));
    }

    Ok(out_path.to_string_lossy().to_string())
}

pub fn start_export(
    db: DbPool,
    job_mgr: Arc<JobManager>,
    app: AppHandle,
    project_id: String,
    opts: ExportOptions,
    resource_dir: Option<PathBuf>,
) -> Result<Value, String> {
    if opts.target == "ssh" { export_profiles(&opts)?; }
    else if !opts.target.is_empty() && opts.target != "local" { return Err("Tujuan ekspor tidak dikenal".into()); }
    let job = {
        let conn = db.lock().unwrap();
        if let Some(active) = crate::db::repo::active_project_job(&conn, &project_id, "export") { return Ok(active); }
        insert_job(
            &conn,
            &project_id,
            "export",
            "ffmpeg",
            serde_json::to_value(&opts).unwrap_or_default(),
            None,
            None,
        )?
    };

    let j_id = job.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let _ = app.emit("event:job", &job);

    let db_clone = db.clone();
    let p_id = project_id.clone();
    let res_dir = resource_dir.clone();

    job_mgr.spawn_task(&j_id, QueueName::Export, move |ctx| async move {
        let out = render_project(&ctx, &p_id, &opts, &db_clone, res_dir.as_deref()).await?;
        {
            let conn = db_clone.lock().unwrap();
            let _ = touch_project(&conn, &p_id, Some("exported"));
        }
        Ok(Some(out))
    });

    Ok(job)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narration_concatenates_padded_clips_and_silent_gaps_without_delay_buffers() {
        let segments = plan_segments(&[clip(300,"fade"),clip(200,"cut"),clip(400,"cut")],30);
        let graph = narration_graph(&segments,&[Some(0),None,Some(1)]);
        assert!(graph[0].contains("apad,atrim=duration=0.300000,asetpts=PTS-STARTPTS[a0]"));
        assert!(graph[1].starts_with("anullsrc=r=48000:cl=stereo,atrim=duration=0.200000"));
        assert!(graph[2].starts_with("[1:a]"));
        assert_eq!(graph[3],"[a0][a1][a2]concat=n=3:v=0:a=1[voice]");
        assert!(!graph.join(";").contains("adelay"));
        assert!(narration_graph(&segments,&[None,None,None]).is_empty());
    }

    #[test]
    #[ignore = "writes synthetic audio graphs for the optional bounded-memory Linux test"]
    fn write_narration_memory_fixture() {
        let directory = PathBuf::from(std::env::var_os("DE_STORY_RENDER_FIXTURE_DIR").expect("fixture directory"));
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(".toolchain").canonicalize().unwrap();
        assert_eq!(directory.parent().unwrap().canonicalize().unwrap(),workspace);
        assert!(directory.file_name().unwrap().to_string_lossy().starts_with("render-memory-"));
        fs::create_dir_all(&directory).unwrap();
        let long = plan_segments(&(0..60).map(|_|clip(10000,"cut")).collect::<Vec<_>>(),30);
        let short = plan_segments(&[clip(300,"cut"),clip(200,"cut"),clip(400,"cut")],30);
        let fixture = serde_json::json!({"long":narration_graph(&long,&(0..60).map(Some).collect::<Vec<_>>()).join(";\n"),"short":narration_graph(&short,&[Some(0),None,Some(1)]).join(";\n")});
        fs::write(directory.join("graphs.json"),serde_json::to_vec(&fixture).unwrap()).unwrap();
    }

    #[test]
    fn distributed_render_balances_duration_and_preserves_every_clip_once() {
        let segs = plan_segments(&[clip(1000,"fade"), clip(9000,"cut"), clip(6000,"fade"), clip(4000,"cut")],30);
        let groups = distribute_segments(&segs,2);
        assert_eq!(groups.len(),2);
        let mut indices = groups.concat(); indices.sort();
        assert_eq!(indices,vec![0,1,2,3]);
        assert_eq!(groups,vec![vec![0,1],vec![2,3]]);
        let first = merged_plan(&segs[..2]);
        let second = merged_plan(&segs[2..]);
        assert_eq!(first.start,0.0);
        assert_eq!(second.start,first.own);
        assert_eq!(first.overlap,segs[1].overlap);
        assert_eq!(second.overlap,0.0);
        assert!((first.own + second.own - segs.iter().map(|s|s.own).sum::<f64>()).abs()<1e-9);
        let loads: Vec<f64> = groups.iter().map(|g|g.iter().map(|&i|segs[i].own+segs[i].overlap).sum()).collect();
        assert!((loads[0]-loads[1]).abs()<3.0);
        assert_eq!(distribute_segments(&segs,20).len(),4);
        assert!(distribute_segments(&[],2).is_empty());
        let mut opts: ExportOptions = serde_json::from_value(serde_json::json!({"fps":30,"resolution":1440,"burnCaptions":true,"writeSrt":false,"duckMusic":true,"normalizeAudio":true,"folder":"test","fileName":"test","sshProfile":"primary","sshProfiles":["other","PRIMARY","third","other"]})).unwrap();
        assert_eq!(export_profiles(&opts).unwrap(),vec!["primary","other","third"]);
        opts.ssh_profiles.push("bad;hostname".into()); assert!(export_profiles(&opts).is_err());
    }

    fn clip(duration_ms: i64, transition: &str) -> ClipRow {
        ClipRow {
            id: String::new(),
            project_id: String::new(),
            sort: 0,
            title: String::new(),
            story: String::new(),
            visual_prompt: String::new(),
            visual_source: None,
            narration: String::new(),
            duration_ms,
            character_ids: Vec::new(),
            motion_type: "camera".to_string(),
            camera_preset: "zoomin".to_string(),
            motion_strength: "halus".to_string(),
            video_prompt: String::new(),
            transition: transition.to_string(),
            image_asset_id: None,
            video_asset_id: None,
            audio_asset_id: None,
        }
    }

    #[test]
    fn cuts_concatenate_all_clips_at_frame_boundaries() {
        let clips = [clip(5429, "cut"), clip(6872, "cut"), clip(7337, "cut")];
        let segs = plan_segments(&clips, 60);
        assert_eq!(segs[0].own, 326.0 / 60.0);
        assert_eq!(segs[1].start, 326.0 / 60.0);
        assert_eq!(segs[2].start, 738.0 / 60.0);
        assert_eq!(segs.iter().map(|s| s.own).sum::<f64>(), 1178.0 / 60.0);
        assert!(segs.iter().all(|s| s.overlap == 0.0));

        let (graph, output) = video_graph(&segs, 60);
        assert_eq!(output, "[vbase]");
        assert_eq!(graph[0], "[0:v][1:v][2:v]concat=n=3:v=1:a=0,settb=AVTB[block0]");
        assert!(graph.iter().all(|line| !line.contains("xfade")));
    }

    #[test]
    fn camera_motion_uses_chroma_safe_working_format() {
        let size = OutputSize { w: 1920, h: 1080 };
        let vf = still_image_filter(&size, 1080, 30, "zoomin", "halus", 150);
        assert!(vf.contains("format=yuv444p,zoompan="));
        assert!(vf.contains("setsar=1,format=yuv420p"));
        assert!(vf.contains("scale=3840:2160"));
        let vf_4k = still_image_filter(&OutputSize { w: 3840, h: 2160 }, 2160, 30, "zoomin", "halus", 150);
        assert!(vf_4k.contains("scale=5760:3240")); // 1.5x at 4k
        for (preset, strength, frames, fps) in [("zoomin", "halus", 300, 30), ("zoomout", "kuat", 900, 60), ("kenburns", "sedang", 600, 30)] {
            let motion = camera_zoompan(preset, strength, frames, fps);
            assert!(motion.z.contains("min(on/"));
            assert!(motion.x.contains("iw-iw/zoom"));
            assert!(motion.y.contains("ih-ih/zoom"));
        }
    }

    #[test]
    fn mixed_cuts_and_fades_use_frame_aligned_xfade() {
        let clips = [
            clip(3000, "cut"),
            clip(4000, "fade"),
            clip(5000, "cut"),
            clip(3500, "zoomin"),
            clip(4500, "cut"),
            clip(3000, "fade"),
        ];
        let segs = plan_segments(&clips, 60);
        let (graph, output) = video_graph(&segs, 60);
        assert_eq!(output, "[vbase]");
        assert_eq!(graph[0], "[0:v][1:v]concat=n=2:v=1:a=0,settb=AVTB[block0]");
        assert_eq!(graph[1], "[2:v][3:v]concat=n=2:v=1:a=0,settb=AVTB[block1]");
        assert_eq!(graph[2], "[4:v][5:v]concat=n=2:v=1:a=0,settb=AVTB[block2]");
        assert_eq!(
            graph[3],
            "[block0][block1]xfade=transition=fade:duration=0.400000:offset=7.000000[x0]"
        );
        assert_eq!(
            graph[4],
            "[x0][block2]xfade=transition=zoomin:duration=0.400000:offset=15.500000[x1]"
        );
    }

    #[test]
    fn anchor_offset_matches_preview_nine_points() {
        // Preview anchor points: 1=bottom-left, 2=bottom-center, 3=bottom-right
        // 4=middle-left, 5=center, 6=middle-right
        // 7=top-left, 8=top-center, 9=top-right
        let a1 = anchor_offset(1);
        assert_eq!((a1.ax, a1.ay), (0.0, 1.0));
        let a5 = anchor_offset(5);
        assert_eq!((a5.ax, a5.ay), (0.5, 0.5));
        let a7 = anchor_offset(7);
        assert_eq!((a7.ax, a7.ay), (0.0, 0.0));
        let a9 = anchor_offset(9);
        assert_eq!((a9.ax, a9.ay), (1.0, 0.0));
    }

    #[test]
    fn output_size_handles_aspect_ratios() {
        let l1080 = output_size("16:9", 1080);
        assert_eq!((l1080.w, l1080.h), (1920, 1080));
        let p1080 = output_size("9:16", 1080);
        assert_eq!((p1080.w, p1080.h), (1080, 1920));
        let l2k = output_size("16:9", 1440);
        assert_eq!((l2k.w, l2k.h), (2560, 1440));
        let p2k = output_size("9:16", 1440);
        assert_eq!((p2k.w, p2k.h), (1440, 2560));
        let l4k = output_size("16:9", 2160);
        assert_eq!((l4k.w, l4k.h), (3840, 2160));
    }

    #[test]
    fn image_overlay_preserves_aspect_ratio_and_uses_actual_dimensions() {
        let size = OutputSize { w: 1920, h: 1080 };
        let overlay = serde_json::json!({
            "width": 0.2,
            "opacity": 0.85,
            "anchor": 9,
            "x": 0.95,
            "y": 0.05,
            "start": 1.0,
            "end": 4.5
        });
        let [scaled, composite] = image_overlay_filters(&overlay, &size, 10.0, 3, 0, "[vbase]");
        // Scale preserves aspect ratio with -1 height
        assert_eq!(scaled, "[3:v]format=rgba,scale=384:-1,setsar=1,colorchannelmixer=aa=0.8500[ov0]");
        // Uses overlay_w and overlay_h in placement expressions, with anchor 9 (top right, ax=1, ay=0)
        assert!(composite.contains("overlay=x='1824.000-overlay_w*1':y='54.000-overlay_h*0'"));
        assert!(composite.contains("enable='gte(t,1.000)*lt(t,4.500)'"));
    }
    #[tokio::test]
    #[ignore = "renders a tiny synthetic 2K fixture locally and on the configured VPS"]
    async fn render_1440p_dimensions_on_local_and_ssh() {
        let profile = std::env::var("DE_STORY_SSH_PROFILE").unwrap_or("instabox-render".into());
        let tmp = tempfile::tempdir().unwrap();
        let token = CancellationToken::new();
        for (aspect, fps) in [("16:9", 30), ("9:16", 60)] {
            let size = output_size(aspect, 1440);
            for ssh in [false, true] {
                let id = uuid::Uuid::new_v4().to_string();
                let mut remote = if ssh { Some(SshExport::new(&profile, &id, tmp.path(), "2K synthetic render check", true, &token).await.unwrap()) } else { None };
                let output = tmp.path().join(format!("2k-{fps}-{ssh}.mp4"));
                let args = vec!["-f".into(), "lavfi".into(), "-i".into(), format!("color=c=navy:s={}x{}:r={fps}", size.w, size.h),
                    "-t".into(), "0.1".into(), "-c:v".into(), "libx264".into(), "-preset".into(), "ultrafast".into(), "-threads".into(), "1".into(),
                    "-pix_fmt".into(), "yuv420p".into(), output.to_string_lossy().to_string()];
                run_ffmpeg(&args, tmp.path(), &token, Some(0.1), |_| {}, None, &mut remote).await.unwrap();
                assert_eq!(crate::services::audio::probe_size(&output).await.unwrap(), (size.w, size.h));
            }
        }
    }

    #[tokio::test]
    #[ignore = "renders 20 synthetic 2K clips with a 3 GiB address-space cap on the configured Linux VPS"]
    async fn ssh_grouped_2k_memory_regression() {
        let profile = std::env::var("DE_STORY_SSH_PROFILE").unwrap_or("instabox-render".into());
        let tmp = tempfile::tempdir().unwrap();
        let token = CancellationToken::new();
        let id = uuid::Uuid::new_v4().to_string();
        let mut remote = Some(SshExport::new(&profile,&id,tmp.path(),"2K grouped memory regression",true,&token).await.unwrap());
        let worker = tmp.path().join("ssh-upload").join(&id).join("worker.py");
        let code = fs::read_to_string(&worker).unwrap();
        fs::write(&worker,format!("import resource\nresource.setrlimit(resource.RLIMIT_STACK,(524288,524288))\nresource.setrlimit(resource.RLIMIT_AS,(3221225472,3221225472))\n{code}")).unwrap();
        let source: Vec<String> = ["-f","lavfi","-i","color=c=navy:s=2560x1440:r=30","-t","1.4","-an","-c:v","libx264","-threads","2","-preset","ultrafast","-pix_fmt","yuv420p","leaf.mp4"].iter().map(|s|s.to_string()).collect();
        remote.as_mut().unwrap().queue(&source,1.4).unwrap();
        let opts: ExportOptions = serde_json::from_value(serde_json::json!({"fps":30,"resolution":1440,"burnCaptions":false,"writeSrt":false,"duckMusic":false,"normalizeAudio":false,"folder":"unused","fileName":"fixture"})).unwrap();
        let plans = plan_segments(&(0..20).map(|i|clip(1000,if i%3==0 {"cut"} else {"fade"})).collect::<Vec<_>>(),30);
        let (file,plan) = merge_video_parts(vec!["leaf.mp4".into();20],plans,700,&opts,tmp.path(),&token,None,&mut remote).await.unwrap();
        assert_eq!(plan.own,20.0); assert_eq!(plan.overlap,0.0);
        let output = tmp.path().join("bounded-2k.mp4");
        let args=vec!["-i".into(),file,"-c:v".into(),"copy".into(),"-an".into(),output.to_string_lossy().to_string()];
        run_ffmpeg(&args,tmp.path(),&token,Some(20.0),|_|{},None,&mut remote).await.unwrap();
        assert_eq!(crate::services::audio::probe_size(&output).await.unwrap(),(2560,1440));
        let duration=crate::services::audio::probe_duration_ms(&output).await.unwrap();
        assert!((duration-20000.0).abs()<100.0,"duration {duration}");
    }

    #[tokio::test]
    #[ignore = "uses the configured VPS to verify SSH upload, render, download and deletion"]
    async fn ssh_distributed_render_round_trip_and_result_management() {
        use uuid::Uuid;
        use crate::media::ssh_export::{self, SshExport};
        let profile = std::env::var("DE_STORY_SSH_PROFILE").unwrap_or("instabox-render".into());
        let temp = tempfile::Builder::new().prefix("ssh render 'test ").tempdir().unwrap();
        let cwd = temp.path();
        let token = CancellationToken::new();
        let source = cwd.join("test source.png");
        let audio = cwd.join("test audio.wav");
        let mut local = None;
        let image_args: Vec<String> = ["-f", "lavfi", "-i", "color=c=navy:s=360x640:r=30", "-frames:v", "1"].iter().map(|s| s.to_string()).chain([source.to_string_lossy().to_string()]).collect();
        run_ffmpeg(&image_args, cwd, &token, None, |_| {}, None, &mut local).await.unwrap();
        let audio_args: Vec<String> = ["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=2", "-c:a", "pcm_s16le"].iter().map(|s| s.to_string()).chain([audio.to_string_lossy().to_string()]).collect();
        run_ffmpeg(&audio_args, cwd, &token, None, |_| {}, None, &mut local).await.unwrap();
        let id = Uuid::new_v4().to_string();
        let mut remote = Some(SshExport::new(&profile, &id, cwd, "SSH test", false, &token).await.unwrap());
        let size = output_size("9:16", 720);
        let segments = plan_segments(&[clip(1000, "fade"), clip(1000, "cut")], 30);
        let second = std::env::var("DE_STORY_SSH_SECOND_PROFILE").unwrap_or(profile.clone());
        let mut nodes = Vec::new();
        for (i, segment) in segments.iter().enumerate() {
            let child_id = Uuid::new_v4().to_string();
            let mut child = SshExport::new(if i==0 {&profile} else {&second},&child_id,cwd,"Distributed synthetic clip",false,&token).await.unwrap();
            let len = segment.own + segment.overlap;
            let filter = still_image_filter(&size, 720, 30, "zoomin", "halus", 30);
            let args = vec!["-loop".into(), "1".into(), "-framerate".into(), "30".into(), "-t".into(), format!("{len}"), "-i".into(), source.to_string_lossy().to_string(), "-vf".into(), filter, "-an".into(), "-c:v".into(), "libx264".into(), "-preset".into(), "veryfast".into(), "-pix_fmt".into(), "yuv420p".into(), "-r".into(), "30".into(), format!("seg_{i:03}.mp4")];
            child.queue(&args,len).unwrap();
            nodes.push((child,vec![format!("seg_{i:03}.mp4")]));
        }
        remote.as_ref().unwrap().prepare_receiver(&token).await.unwrap();
        let results = futures_util::future::join_all(nodes.iter().map(|(node,files)|node.run_segments(files,cwd,remote.as_ref().unwrap(),&token,|_|{}))).await;
        assert!(results.iter().all(Result::is_ok),"{results:?}");
        assert!(!cwd.join("seg_000.mp4").exists() && !cwd.join("seg_001.mp4").exists(),"Intermediate payload must not pass through the laptop");
        for (node,_) in &nodes {
            assert!(!ssh_export::list_results(&profile).await.unwrap()["results"].as_array().unwrap().iter().any(|r|r["id"]==node.id()));
        }
        std::fs::create_dir(cwd.join("fonts")).unwrap();
        let font = std::fs::read_dir(fonts_dir(None)).unwrap().filter_map(Result::ok).find(|p| p.file_name().to_string_lossy().starts_with("Poppins") && p.path().extension().is_some_and(|e| e == "ttf")).unwrap();
        std::fs::copy(font.path(), cwd.join("fonts").join(font.file_name())).unwrap();
        std::fs::write(cwd.join("captions.ass"), "[Script Info]\nScriptType: v4.00+\nPlayResX: 720\nPlayResY: 1280\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Default,Poppins,50,&H00FFFFFF,&H00FFFFFF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,0,2,20,20,60,1\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:00.00,0:00:02.00,Default,,0,0,0,,SSH render test\n").unwrap();
        let (mut graph, video) = video_graph(&segments, 30);
        graph.push(format!("{video}ass=captions.ass:fontsdir=fonts,format=yuv420p[vout]"));
        graph.extend(narration_graph(&segments,&[Some(2),Some(3)]));
        graph.push("[voice]anull[aout]".into());
        std::fs::write(cwd.join("graph.txt"), graph.join(";\n")).unwrap();
        let output = cwd.join("hasil SSH test.mp4");
        let args = vec!["-threads".into(),"1".into(),"-i".into(), "seg_000.mp4".into(), "-threads".into(),"1".into(),"-i".into(), "seg_001.mp4".into(), "-i".into(), audio.to_string_lossy().to_string(), "-i".into(), audio.to_string_lossy().to_string(), "-filter_complex_script".into(), "graph.txt".into(), "-map".into(), "[vout]".into(), "-map".into(), "[aout]".into(), "-c:v".into(), "libx264".into(), "-threads".into(),"2".into(),"-preset".into(), "veryfast".into(), "-c:a".into(), "aac".into(), "-t".into(), "2".into(), output.to_string_lossy().to_string()];
        run_ffmpeg(&args, cwd, &token, Some(2.0), |_| {}, None, &mut remote).await.unwrap();
        assert_eq!(crate::services::audio::probe_size(&output).await.unwrap(), (720, 1280));
        let duration = crate::services::audio::probe_duration_ms(&output).await.unwrap();
        assert!((duration-2000.0).abs()<100.0,"duration {duration}");
        let list = ssh_export::list_results(&profile).await.unwrap();
        assert!(list["results"].as_array().unwrap().iter().any(|r| r["id"] == id && r["state"] == "completed"));
        let downloaded = ssh_export::download_result(&profile, &id, &cwd.join("download"), false).await.unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), std::fs::read(downloaded).unwrap());
        ssh_export::delete_result(&profile, &id).await.unwrap();
        assert!(!ssh_export::list_results(&profile).await.unwrap()["results"].as_array().unwrap().iter().any(|r| r["id"] == id));
        let id = Uuid::new_v4().to_string();
        let mut remote = Some(SshExport::new(&profile, &id, cwd, "Auto-delete test", true, &token).await.unwrap());
        let output = cwd.join("auto-delete.mp4");
        let args = vec!["-loop".into(), "1".into(), "-framerate".into(), "30".into(), "-i".into(), source.to_string_lossy().to_string(), "-t".into(), "0.3".into(), "-vf".into(), "format=yuv420p".into(), "-c:v".into(), "libx264".into(), "-preset".into(), "ultrafast".into(), output.to_string_lossy().to_string()];
        run_ffmpeg(&args, cwd, &token, Some(0.3), |_| {}, None, &mut remote).await.unwrap();
        assert!(output.is_file());
        assert!(!ssh_export::list_results(&profile).await.unwrap()["results"].as_array().unwrap().iter().any(|r| r["id"] == id));
    }

}

