use crate::{
    paths::ffmpeg_path,
    secrets::KeyTestResult,
    speech::align::{align_words, norm, split_heard, Heard, WordTiming},
};
use futures_util::{SinkExt, StreamExt};
use reqwest::{
    multipart::{Form, Part},
    Client,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    time::Duration,
};
use tokio::process::Command;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const BASE: &str = "https://router.speko.dev";
const WS: &str = "wss://router.speko.dev/v1/tts/stream";

fn client() -> Result<Client, String> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|_| "Klien Speko tidak dapat dibuat".into())
}

fn check_key(key: &str) -> Result<(), String> {
    if key.trim().is_empty() || key.contains(['\r', '\n']) {
        return Err("Isi API key Speko di Pengaturan → Layanan API → Speko.".into());
    }
    Ok(())
}

async fn response_json(mut response: reqwest::Response) -> Result<Value, String> {
    let status = response.status();
    // Provider error bodies may echo submitted text or credentials; expose only the status.
    if !status.is_success() {
        return Err(format!(
            "Speko HTTP {}. Periksa kunci, akses model, saldo, dan batas akun di dashboard Speko.",
            status.as_u16()
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Respons Speko terputus")?
    {
        if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
            return Err("Respons Speko terlalu besar".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "Respons Speko bukan JSON yang valid".into())
}

async fn catalog(key: &str, kind: &str) -> Result<Vec<Value>, String> {
    check_key(key)?;
    let mut request = client()?.get(format!("{BASE}/v1/models")).bearer_auth(key);
    if kind == "stt" {
        request = request.query(&[("path", "batch")]);
    }
    let value = response_json(
        request
            .send()
            .await
            .map_err(|_| "Tidak dapat menghubungi katalog Speko".to_string())?,
    )
    .await?;
    value["models"]
        .as_array()
        .cloned()
        .ok_or("Katalog Speko tidak memuat daftar model".into())
}

fn capable(model: &Value, kind: &str) -> bool {
    model["kind"].as_str() == Some(kind)
        && if kind == "tts" {
            model["capabilities"]["word_timings"] == true
                && model["capabilities"]["streaming"] == true
        } else {
            model["capabilities"]["word_timestamps"] == true
        }
}

fn route_id(model: &Value) -> Option<String> {
    let provider = model["provider"].as_str().filter(|v| !v.is_empty())?;
    let id = model["id"].as_str().filter(|v| !v.is_empty())?;
    Some(format!("{provider}/{id}"))
}

fn routing(id: &str) -> Result<Value, String> {
    let (provider, model) = id
        .split_once('/')
        .filter(|(p, m)| !p.is_empty() && !m.is_empty())
        .ok_or("Pilih model Speko dari katalog di Pengaturan terlebih dahulu")?;
    Ok(json!({"mode":"explicit","provider":provider,"model":model}))
}

pub async fn test_key(key: &str) -> KeyTestResult {
    match catalog(key, "tts").await {
        Ok(models) => KeyTestResult {
            ok: true,
            message: Some(format!(
                "Terhubung · {} model dalam katalog akun",
                models.len()
            )),
        },
        Err(message) => KeyTestResult {
            ok: false,
            message: Some(message),
        },
    }
}

pub async fn list_models(key: &str, kind: &str) -> Result<Vec<Value>, String> {
    Ok(catalog(key, kind).await?.iter().filter(|m| capable(m, kind)).filter_map(|m| {
        Some(json!({"id":route_id(m)?,"name":format!("{} · {}",m["id"].as_str()?,m["provider"].as_str()?),
            "tags":[if kind == "tts" { "Waktu kata bawaan" } else { "STT · waktu kata" }]}))
    }).collect())
}

async fn selected(
    key: &str,
    model: &str,
    kind: &str,
    token: &CancellationToken,
) -> Result<Value, String> {
    routing(model)?;
    let models = tokio::select! { _ = token.cancelled() => return Err("Dibatalkan".into()), r = catalog(key, kind) => r? };
    models.into_iter().find(|m| capable(m, kind) && route_id(m).as_deref() == Some(model))
        .ok_or("Model Speko tidak lagi tersedia atau tidak mendukung waktu kata. Muat ulang katalog lalu pilih model lain.".into())
}

fn parse_spans(items: &Value, duration: f64) -> Result<Vec<Heard>, String> {
    let rows = items
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or("Speko tidak mengirim waktu kata".to_string())?;
    let mut words = Vec::new();
    for row in rows {
        let text = row["text"]
            .as_str()
            .ok_or("Teks waktu kata Speko tidak valid")?;
        let start = row["start_ms"]
            .as_f64()
            .ok_or("Waktu awal Speko tidak valid")?
            / 1000.0;
        let end = row["end_ms"]
            .as_f64()
            .ok_or("Waktu akhir Speko tidak valid")?
            / 1000.0;
        if !start.is_finite()
            || !end.is_finite()
            || start < 0.0
            || end < start
            || start > duration + 0.25
            || end > duration + 0.25
        {
            return Err("Waktu kata Speko berada di luar audio".into());
        }
        if !norm(text).is_empty() {
            let start = start.min((duration - 0.001).max(0.0));
            words.push(Heard {
                text: text.into(),
                start,
                end: end.max(start + 0.001).min(duration),
            });
        }
    }
    // Overlapping/zero-duration spans are legal; keep chronological words within the actual audio.
    words.sort_by(|a, b| a.start.total_cmp(&b.start));
    let words = split_heard(&words);
    if words.is_empty() {
        return Err("Speko tidak mengirim kata yang dapat diberi waktu".into());
    }
    Ok(words)
}

fn aligned(script: &[String], heard: &[Heard], duration: f64) -> Result<Vec<WordTiming>, String> {
    let mut counts = HashMap::<String, usize>::new();
    for word in heard {
        *counts.entry(norm(&word.text)).or_default() += 1;
    }
    let matched = script
        .iter()
        .filter(|w| {
            if let Some(n) = counts.get_mut(&norm(w)) {
                if *n > 0 {
                    *n -= 1;
                    return true;
                }
            }
            false
        })
        .count();
    if script.is_empty() || (matched as f64 / script.len() as f64) < 0.65 {
        return Err("Transkrip Speko terlalu berbeda dari naskah. Periksa bahasa/model atau edit caption secara manual.".into());
    }
    Ok(align_words(script, heard, duration))
}

pub async fn speak(
    key: &str,
    model: &str,
    voice: &str,
    text: &str,
    language: &str,
    token: &CancellationToken,
) -> Result<(Vec<u8>, Vec<WordTiming>), String> {
    if text.trim().is_empty() || text.chars().count() > 10_000 {
        return Err("Teks Speko TTS harus berisi 1–10.000 karakter per klip".into());
    }
    selected(key, model, "tts", token).await?;
    speak_at(WS, key, model, voice, text, language, token).await
}

async fn speak_at(
    url: &str,
    key: &str,
    model: &str,
    voice: &str,
    text: &str,
    language: &str,
    token: &CancellationToken,
) -> Result<(Vec<u8>, Vec<WordTiming>), String> {
    check_key(key)?;
    let mut request = url
        .into_client_request()
        .map_err(|_| "Alamat Speko tidak valid")?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {key}")
            .parse()
            .map_err(|_| "Kunci Speko tidak valid")?,
    );
    request.headers_mut().insert(
        "Idempotency-Key",
        Uuid::new_v4().to_string().parse().unwrap(),
    );
    let operation = async {
        let (mut socket, _) = connect_async(request).await.map_err(|_| {
            "Koneksi Speko TTS gagal. Periksa akses model, kunci, dan saldo akun.".to_string()
        })?;
        let mut configure = json!({"type":"session.configure","routing":routing(model)?,"language":language,
            "audio":{"encoding":"pcm_s16le","sample_rate_hz":24000,"channels":1}});
        if !voice.is_empty() && voice != "auto" {
            configure["voice"] = json!(voice);
        }
        socket
            .send(Message::Text(configure.to_string().into()))
            .await
            .map_err(|_| "Gagal mengatur sesi Speko")?;
        let (mut ready, mut started, mut done) = (false, false, false);
        let mut pcm = Vec::new();
        let mut spans = Vec::new();
        while let Some(message) = socket.next().await {
            match message.map_err(|_| "Koneksi Speko terputus sebelum hasil lengkap")? {
                Message::Text(frame) => {
                    let event: Value =
                        serde_json::from_str(&frame).map_err(|_| "Event Speko tidak valid")?;
                    match event["type"].as_str().unwrap_or("") {
                        "session.ready" if !ready => {
                            ready = true;
                            for frame in [json!({"type":"input.append","text":text}), json!({"type":"input.commit"})] {
                                socket.send(Message::Text(frame.to_string().into())).await.map_err(|_| "Gagal mengirim teks Speko")?;
                            }
                        }
                        "utterance.started" if ready && !started && event["sequence"] == 1 => started = true,
                        "utterance.timings" if started && !done && event["sequence"] == 1 && event["granularity"] == "word" => {
                            spans.extend(event["spans"].as_array().ok_or("Event waktu kata Speko tidak valid")?.iter().cloned());
                            if spans.len() > 20_000 { return Err("Terlalu banyak waktu kata Speko".into()); }
                        }
                        "utterance.done" if started && !done && event["sequence"] == 1 => {
                            done = true;
                            socket.send(Message::Text(json!({"type":"session.close"}).to_string().into())).await.map_err(|_| "Gagal menutup sesi Speko")?;
                        }
                        // Usage metadata can arrive before, during, or after an utterance.
                        "usage.updated" if ready && event["usage"].is_object() => {}
                        "session.closed" if done => {
                            if pcm.is_empty() || pcm.len() % 2 != 0 { return Err("Audio Speko kosong atau tidak lengkap".into()); }
                            let duration = pcm.len() as f64 / 48000.0;
                            let words = parse_spans(&json!(spans), duration)?.into_iter().map(|w| WordTiming { text:w.text,start:w.start,end:w.end }).collect();
                            return Ok((super::audio::pcm_to_wav(&pcm,24000,1,16),words));
                        }
                        "error" | "session.error" => return Err("Speko menolak sintesis. Periksa model, voice ID, bahasa, saldo, dan batas akun di dashboard.".into()),
                        kind => {
                            let name: String = kind.chars().filter(|c| c.is_ascii_alphanumeric() || "._-".contains(*c)).take(48).collect();
                            return Err(format!("Event Speko TTS `{name}` tidak didukung atau urutannya tidak valid; hasil tidak lengkap tidak disimpan"));
                        }
                    }
                }
                Message::Binary(bytes) if started && !done => {
                    if pcm.len() + bytes.len() > 48_000 * 600 {
                        return Err("Audio Speko melebihi batas 10 menit per klip".into());
                    }
                    pcm.extend_from_slice(&bytes);
                }
                Message::Ping(bytes) => {
                    socket
                        .send(Message::Pong(bytes))
                        .await
                        .map_err(|_| "Speko terputus")?;
                }
                Message::Pong(_) => {}
                _ => return Err("Sesi Speko berakhir sebelum hasil lengkap".into()),
            }
        }
        Err("Speko tidak mengirim hasil lengkap".into())
    };
    tokio::select! {
        _ = token.cancelled() => Err("Dibatalkan; karakter yang sudah diterima Speko dapat tetap ditagih".into()),
        result = tokio::time::timeout(Duration::from_secs(600), operation) => result.map_err(|_| "Speko TTS melewati batas waktu. Periksa penggunaan di dashboard sebelum mengulang.".to_string())?,
    }
}

fn chunk_seconds(model: &Value) -> Result<f64, String> {
    let limits = &model["batch_audio_limits"];
    let seconds = 45.0_f64
        .min(limits["max_duration_seconds"].as_f64().unwrap_or(45.0))
        .min(limits["max_pcm_bytes"].as_f64().unwrap_or(45.0 * 32000.0) / 32000.0);
    if !seconds.is_finite() || seconds < 3.0 {
        return Err(
            "Batas unggah model Speko terlalu kecil untuk caption. Pilih model lain.".into(),
        );
    }
    Ok(seconds)
}

async fn transcribe_at(
    base: &str,
    key: &str,
    model: &str,
    language: &str,
    pcm: &[u8],
) -> Result<Vec<Heard>, String> {
    let metadata =
        json!({"routing":routing(model)?,"language":language,"options":{"word_timestamps":true}});
    // Router requires metadata before the audio part, and an idempotency key for every paid request.
    let form = Form::new()
        .part(
            "request",
            Part::text(metadata.to_string())
                .mime_str("application/json")
                .unwrap(),
        )
        .part(
            "audio",
            Part::bytes(super::audio::pcm_to_wav(pcm, 16000, 1, 16))
                .file_name("caption.wav")
                .mime_str("audio/wav")
                .unwrap(),
        );
    let response = client()?
        .post(format!("{base}/v1/stt/transcriptions"))
        .bearer_auth(key)
        .header("Idempotency-Key", Uuid::new_v4().to_string())
        .multipart(form)
        .send()
        .await
        .map_err(|_| {
            "Speko STT tidak merespons. Periksa penggunaan di dashboard sebelum mengulang."
                .to_string()
        })?;
    let value = response_json(response).await?;
    if value["text"].as_str().is_some_and(|t| t.trim().is_empty())
        && value["words"].as_array().is_some_and(|w| w.is_empty())
    {
        return Ok(Vec::new());
    }
    parse_spans(&value["words"], pcm.len() as f64 / 32000.0)
}

async fn transcribe_pcm(
    base: &str,
    key: &str,
    model: &str,
    language: &str,
    path: &Path,
    chunk: f64,
) -> Result<(Vec<Heard>, f64), String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let bytes = file.metadata().map_err(|e| e.to_string())?.len();
    let duration = bytes as f64 / 32000.0;
    if duration <= 0.0 || duration > 7200.0 {
        return Err("Audio Speko harus berdurasi lebih dari 0 dan maksimal 120 menit".into());
    }
    let mut heard = Vec::new();
    let mut start = 0.0_f64;
    loop {
        let end = (start + chunk).min(duration);
        let first = (start * 16000.0).round() as u64 * 2;
        let last = ((end * 16000.0).round() as u64 * 2).min(bytes);
        file.seek(SeekFrom::Start(first))
            .map_err(|e| e.to_string())?;
        let mut pcm = vec![0; (last - first) as usize];
        file.read_exact(&mut pcm).map_err(|e| e.to_string())?;
        let words = transcribe_at(base, key, model, language, &pcm).await?;
        // One-second overlap keeps words at upload boundaries intact; assign each seam once.
        for mut w in words {
            w.start += start;
            w.end += start;
            let mid = (w.start + w.end) / 2.0;
            if (start == 0.0 || mid >= start + 0.5) && (end == duration || mid < end - 0.5) {
                heard.push(w);
            }
        }
        if end >= duration {
            break;
        }
        start = end - 1.0;
    }
    heard.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok((heard, duration))
}

pub async fn time_script(
    key: &str,
    model: &str,
    resource_dir: Option<&Path>,
    audio: &Path,
    script: &[String],
    language: &str,
    _total: f64,
    tmp: &Path,
    token: &CancellationToken,
) -> Result<Vec<WordTiming>, String> {
    let model_info = selected(key, model, "stt", token).await?;
    let chunk = chunk_seconds(&model_info)?;
    let path = tmp.join(format!("speko-{}.pcm", Uuid::new_v4()));
    let result = async {
        let mut command = Command::new(ffmpeg_path(resource_dir));
        command
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(audio)
            .args(["-t", "7201", "-ar", "16000", "-ac", "1", "-f", "s16le"])
            .arg(&path)
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let output = command
            .output()
            .await
            .map_err(|_| "FFmpeg tidak dapat menyiapkan audio untuk Speko")?;
        if !output.status.success() {
            return Err("Konversi audio Speko gagal".into());
        }
        let (heard, duration) = transcribe_pcm(BASE, key, model, language, &path, chunk).await?;
        aligned(script, &heard, duration)
    };
    let result =
        tokio::select! { _ = token.cancelled() => Err("Dibatalkan".into()), r = result => r };
    let _ = std::fs::remove_file(path);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_requires_real_word_timings_and_retains_nested_model_id() {
        let model = json!({"id":"org/model:v2","provider":"vendor","kind":"tts","capabilities":{"streaming":true,"word_timings":true}});
        assert!(capable(&model, "tts"));
        assert!(!capable(&model, "stt"));
        assert!(!capable(
            &json!({"kind":"tts","capabilities":{"character_timings":true,"streaming":true}}),
            "tts"
        ));
        assert_eq!(
            routing(&route_id(&model).unwrap()).unwrap()["model"],
            "org/model:v2"
        );
        assert!(routing("").is_err());
    }
    #[test]
    fn validates_limits_and_legal_overlapping_timings() {
        assert_eq!(
            chunk_seconds(&json!({"batch_audio_limits":{"max_pcm_bytes":320000}})).unwrap(),
            10.0
        );
        assert!(chunk_seconds(&json!({"batch_audio_limits":{"max_duration_seconds":1}})).is_err());
        let words = parse_spans(&json!([{"text":"Halo","start_ms":0,"end_ms":500},{"text":"dunia","start_ms":400,"end_ms":400}]),1.0).unwrap();
        assert_eq!(words.len(), 2);
        assert!(words[1].end > words[1].start);
        assert!(parse_spans(&json!([{"text":"salah","start_ms":100,"end_ms":2100}]), 1.0).is_err());
        assert!(aligned(&["naskah".into()], &words, 1.0).is_err());
    }

    #[tokio::test]
    async fn tts_websocket_auth_timings_usage_and_complete_close() {
        use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/v1/tts/stream", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_hdr_async(
                stream,
                |request: &Request, response: Response| {
                    assert_eq!(request.headers()["authorization"], "Bearer test-key");
                    assert!(request.headers().contains_key("Idempotency-Key"));
                    Ok(response)
                },
            )
            .await
            .unwrap();
            let config: Value =
                serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(config["language"], "id");
            assert_eq!(config["audio"]["encoding"], "pcm_s16le");
            assert!(config.get("voice").is_none());
            ws.send(Message::Text(
                json!({"type":"session.ready"}).to_string().into(),
            ))
            .await
            .unwrap();
            let append: Value =
                serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(append["text"], "Halo dunia");
            assert!(ws
                .next()
                .await
                .unwrap()
                .unwrap()
                .to_text()
                .unwrap()
                .contains("input.commit"));
            ws.send(Message::Text(
                json!({"type":"usage.updated","usage":{"characters":10}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
            ws.send(Message::Text(
                json!({"type":"utterance.started","sequence":1})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
            ws.send(Message::Binary(vec![0; 48000].into()))
                .await
                .unwrap();
            for (text, start, end) in [("Halo", 0, 500), ("dunia", 450, 950)] {
                ws.send(Message::Text(json!({"type":"utterance.timings","sequence":1,"granularity":"word","spans":[{"text":text,"start_ms":start,"end_ms":end}]}).to_string().into())).await.unwrap();
                ws.send(Message::Text(
                    json!({"type":"usage.updated","usage":{"characters":10}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            }
            ws.send(Message::Text(
                json!({"type":"utterance.done","sequence":1})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
            assert!(ws
                .next()
                .await
                .unwrap()
                .unwrap()
                .to_text()
                .unwrap()
                .contains("session.close"));
            ws.send(Message::Text(
                json!({"type":"usage.updated","usage":{"characters":10}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
            ws.send(Message::Text(
                json!({"type":"session.closed","usage":{}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        });
        let (wav, words) = speak_at(
            &url,
            "test-key",
            "test/model",
            "auto",
            "Halo dunia",
            "id",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(wav.len(), 48044);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(words.len(), 2);
        assert_eq!(words[1].start, 0.45);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn incomplete_or_cancelled_tts_never_returns_partial_audio() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            ws.next().await.unwrap().unwrap();
            ws.send(Message::Text(
                json!({"type":"session.ready"}).to_string().into(),
            ))
            .await
            .unwrap();
            ws.next().await.unwrap().unwrap();
            ws.next().await.unwrap().unwrap();
            ws.send(Message::Text(
                json!({"type":"utterance.started","sequence":1})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
            ws.send(Message::Binary(vec![0; 4800].into()))
                .await
                .unwrap();
            ws.send(Message::Text(
                json!({"type":"usage.updated","usage":{"characters":4}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        });
        let error = speak_at(
            &url,
            "test",
            "vendor/model",
            "",
            "Halo",
            "id",
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert!(error.contains("lengkap"));
        assert!(!error.contains("Event Speko TTS"));
        server.await.unwrap();
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(
            speak_at(&url, "test", "vendor/model", "", "Halo", "id", &cancelled)
                .await
                .unwrap_err()
                .contains("Dibatalkan")
        );
    }

    #[tokio::test]
    async fn stt_multipart_metadata_precedes_wav_and_requests_word_timestamps() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut data = Vec::new();
            let mut buffer = [0u8; 8192];
            loop {
                let n = stream.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                data.extend_from_slice(&buffer[..n]);
                if let Some(end) = data.windows(4).position(|s| s == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&data[..end]).to_lowercase();
                    let size: usize = headers
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if data.len() >= end + 4 + size {
                        break;
                    }
                }
            }
            let body = String::from_utf8_lossy(&data);
            assert!(body.contains("Bearer dummy-key"));
            assert!(body.to_lowercase().contains("idempotency-key:"));
            assert!(body.find("name=\"request\"").unwrap() < body.find("name=\"audio\"").unwrap());
            assert!(body.contains("\"word_timestamps\":true"));
            assert!(body.contains("RIFF"));
            assert!(body.contains("\"language\":\"id\""));
            let json = json!({"text":"Halo","words":[{"text":"Halo","start_ms":100,"end_ms":800}]})
                .to_string();
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",json.len(),json).as_bytes()).await.unwrap();
        });
        let words = transcribe_at(&base, "dummy-key", "vendor/model", "id", &vec![0; 32000])
            .await
            .unwrap();
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].start, 0.1);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn long_stt_offsets_chunks_and_keeps_overlap_words_once() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut ids = Vec::new();
            for i in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut data = Vec::new();
                let mut buffer = [0u8; 16384];
                loop {
                    let n = stream.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    data.extend_from_slice(&buffer[..n]);
                    if let Some(end) = data.windows(4).position(|s| s == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&data[..end]).to_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length: "))
                            .unwrap()
                            .parse()
                            .unwrap();
                        if data.len() >= end + 4 + length {
                            ids.push(
                                headers
                                    .lines()
                                    .find(|l| l.starts_with("idempotency-key:"))
                                    .unwrap()
                                    .to_string(),
                            );
                            break;
                        }
                    }
                }
                let words = if i == 0 {
                    json!([{"text":"awal","start_ms":100,"end_ms":400},{"text":"batas","start_ms":44000,"end_ms":44400},{"text":"sesudah","start_ms":44600,"end_ms":44900}])
                } else {
                    json!([{"text":"batas","start_ms":0,"end_ms":400},{"text":"sesudah","start_ms":600,"end_ms":900},{"text":"akhir","start_ms":1800,"end_ms":2500}])
                };
                let body = json!({"text":"audio demo","words":words}).to_string();
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
            }
            assert_ne!(ids[0], ids[1]);
        });
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("long.pcm");
        std::fs::write(&path, vec![0; 47 * 32000]).unwrap();
        let (words, duration) = transcribe_pcm(&base, "dummy-key", "test/model", "id", &path, 45.0)
            .await
            .unwrap();
        assert_eq!(duration, 47.0);
        assert_eq!(
            words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(),
            vec!["awal", "batas", "sesudah", "akhir"]
        );
        assert!((words[2].start - 44.6).abs() < 0.001);
        assert!((words[3].start - 45.8).abs() < 0.001);
        server.await.unwrap();
    }
}
