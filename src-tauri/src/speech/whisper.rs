use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::paths::{ffmpeg_path, whisper_cli_path, whisper_model_path};
use crate::speech::align::{align_words, split_heard, Heard, WordTiming};

const _MODEL_FILE: &str = "ggml-small.bin";
const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin";
const MODEL_BYTES: u64 = 487_601_967;
const MODEL_SHA256: &str = "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhisperStatus {
    pub state: String, // "ready" | "downloading" | "verifying" | "missing"
    pub received: u64,
    pub total: u64,
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct WhisperManager {
    is_downloading: Arc<AtomicBool>,
    cancel_requested: Arc<AtomicBool>,
    last_error: Arc<Mutex<Option<String>>>,
    phase: Arc<Mutex<String>>,
    received: Arc<Mutex<u64>>,
}

impl WhisperManager {
    pub fn new() -> Self {
        Self {
            is_downloading: Arc::new(AtomicBool::new(false)),
            cancel_requested: Arc::new(AtomicBool::new(false)),
            last_error: Arc::new(Mutex::new(None)),
            phase: Arc::new(Mutex::new("missing".into())),
            received: Arc::new(Mutex::new(0)),
        }
    }

    pub fn status(&self) -> WhisperStatus {
        let model_path = whisper_model_path();
        let part_path = PathBuf::from(format!("{}.part", model_path.display()));

        if self.is_downloading.load(Ordering::SeqCst) {
            return WhisperStatus {
                state: self.phase.lock().unwrap().clone(),
                received: *self.received.lock().unwrap(),
                total: MODEL_BYTES,
                error: None,
            };
        }

        let ready = model_path.exists() && model_path.metadata().map(|m| m.len()).unwrap_or(0) == MODEL_BYTES;
        let part_size = part_path.metadata().map(|m| m.len()).unwrap_or(0);

        WhisperStatus {
            state: if ready { "ready".into() } else { "missing".into() },
            received: if ready { MODEL_BYTES } else { part_size },
            total: MODEL_BYTES,
            error: if ready { None } else { self.last_error.lock().unwrap().clone() },
        }
    }

    pub fn is_ready(&self, resource_dir: Option<&Path>) -> bool {
        let m = self.status();
        let cli = whisper_cli_path(resource_dir);
        m.state == "ready" && cli.exists()
    }

    pub fn cancel_download(&self) {
        if self.is_downloading.load(Ordering::SeqCst) {
            self.cancel_requested.store(true, Ordering::SeqCst);
        }
    }

    pub fn remove_model(&self, app: &AppHandle) {
        self.cancel_download();
        let m = whisper_model_path();
        let part = PathBuf::from(format!("{}.part", m.display()));
        let _ = fs::remove_file(m);
        let _ = fs::remove_file(part);
        *self.last_error.lock().unwrap() = None;
        let _ = app.emit("event:whisper", self.status());
    }

    pub async fn download_model(&self, app: AppHandle) -> Result<(), String> {
        if self.is_downloading.load(Ordering::SeqCst) {
            return Ok(());
        }

        let m_path = whisper_model_path();
        if m_path.exists() && m_path.metadata().map(|m| m.len()).unwrap_or(0) == MODEL_BYTES {
            return Ok(());
        }

        let part_path = PathBuf::from(format!("{}.part", m_path.display()));
        let have = part_path.metadata().map(|m| m.len()).unwrap_or(0);

        self.is_downloading.store(true, Ordering::SeqCst);
        self.cancel_requested.store(false, Ordering::SeqCst);
        *self.phase.lock().unwrap() = "downloading".into();
        *self.received.lock().unwrap() = have;
        *self.last_error.lock().unwrap() = None;

        let _ = app.emit("event:whisper", self.status());

        let res = self.run_download_loop(&app, &part_path, &m_path, have).await;

        self.is_downloading.store(false, Ordering::SeqCst);
        if let Err(ref e) = res {
            if !self.cancel_requested.load(Ordering::SeqCst) {
                *self.last_error.lock().unwrap() = Some(e.clone());
            }
        }
        let _ = app.emit("event:whisper", self.status());
        res
    }

    async fn run_download_loop(
        &self,
        app: &AppHandle,
        part_path: &Path,
        final_path: &Path,
        mut have: u64,
    ) -> Result<(), String> {
        let client = reqwest::Client::new();
        let mut req = client.get(MODEL_URL);
        if have > 0 {
            req = req.header("Range", format!("bytes={have}-"));
        }

        let mut resp = req.send().await.map_err(|e| format!("Unduhan gagal: {e}"))?;
        let status = resp.status();
        let is_partial = status == reqwest::StatusCode::PARTIAL_CONTENT;

        if !status.is_success() {
            return Err(format!("Unduhan gagal (HTTP {status})"));
        }

        if !is_partial && have > 0 {
            have = 0;
            *self.received.lock().unwrap() = 0;
        }

        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(is_partial)
            .truncate(!is_partial)
            .open(part_path)
            .map_err(|e| e.to_string())?;

        let mut last_emit = Instant::now();

        while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
            if self.cancel_requested.load(Ordering::SeqCst) {
                return Err("Dibatalkan".into());
            }
            file.write_all(&chunk).map_err(|e| e.to_string())?;
            have += chunk.len() as u64;
            *self.received.lock().unwrap() = have;

            if last_emit.elapsed() > Duration::from_millis(250) {
                last_emit = Instant::now();
                let _ = app.emit("event:whisper", self.status());
            }
        }

        file.flush().map_err(|e| e.to_string())?;
        drop(file);

        // Verification phase
        *self.phase.lock().unwrap() = "verifying".into();
        let _ = app.emit("event:whisper", self.status());

        let sha = calculate_sha256(part_path)?;
        if sha != MODEL_SHA256 {
            let _ = fs::remove_file(part_path);
            return Err("File model rusak atau tidak lengkap. Coba unduh lagi.".into());
        }

        fs::rename(part_path, final_path).map_err(|e| e.to_string())?;
        *self.phase.lock().unwrap() = "ready".into();
        Ok(())
    }
}

fn calculate_sha256(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn whisper_lang(code: &str) -> &str {
    if code == "jv" { "jw" } else { code }
}

#[derive(Clone)]
pub struct WhisperExecution {
    pub ssh_profile: Option<String>,
}

impl WhisperExecution {
    pub fn from_settings(settings: &serde_json::Value) -> Result<Self, String> {
        let ssh_profile = match settings["whisperTarget"].as_str().unwrap_or("local") {
            "local" => None,
            "ssh" => {
                let profile = settings["whisperSshProfile"].as_str().filter(|s| !s.is_empty()).or_else(|| settings["exportSshProfile"].as_str()).unwrap_or("instabox-render");
                crate::media::ssh_export::validate_profile(profile)?;
                Some(profile.to_string())
            }
            _ => return Err("Pilihan lokasi Whisper tidak valid".into()),
        };
        Ok(Self { ssh_profile })
    }

    pub fn available(&self, resource_dir: Option<&Path>) -> bool {
        self.ssh_profile.is_some() || whisper_ready_with(resource_dir)
    }

    pub fn message(&self) -> &'static str {
        if self.ssh_profile.is_some() { "Menyelaraskan caption lewat Whisper di VPS" } else { "Menyelaraskan caption lewat Whisper lokal" }
    }
}

async fn run_local(mut command: Command, token: &CancellationToken) -> Result<std::process::Output, String> {
    if token.is_cancelled() { return Err("Dibatalkan".into()); }
    command.kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    tokio::select! {
        _ = token.cancelled() => Err("Dibatalkan".into()),
        output = command.output() => output.map_err(|e| format!("Whisper/FFmpeg gagal dijalankan: {e}")),
    }
}

pub async fn recognize_audio(
    resource_dir: Option<&Path>,
    audio_path: &Path,
    language: &str,
    prompt: &str,
    tmp_dir: &Path,
    token: &CancellationToken,
    execution: &WhisperExecution,
) -> Result<Vec<Heard>, String> {
    // Count characters instead of slicing bytes through a possible Unicode word.
    let prompt_cut: String = prompt.chars().take(400).collect();
    if let Some(profile) = &execution.ssh_profile {
        let json = super::ssh_whisper::recognize(profile, audio_path, whisper_lang(language), &prompt_cut, tmp_dir, token).await?;
        return parse_output(&json);
    }
    let ffmpeg = ffmpeg_path(resource_dir);
    let cli_exe = whisper_cli_path(resource_dir);
    let model = whisper_model_path();

    if !model.exists() || !cli_exe.exists() {
        return Err("Model Whisper belum diunduh. Unduh dulu di Pengaturan.".into());
    }

    let _ = fs::create_dir_all(tmp_dir);
    let run_id = uuid::Uuid::new_v4().to_string();
    let wav_file = tmp_dir.join(format!("whisper-{run_id}.wav"));
    let out_base = tmp_dir.join(format!("whisper-out-{run_id}"));

    // Convert audio to 16kHz mono 16-bit PCM WAV
    let result = async {
        let mut convert = Command::new(&ffmpeg);
        convert
            .args(&[
                "-hide_banner",
                "-loglevel", "error",
                "-y",
                "-i",
                audio_path.to_str().unwrap(),
                "-ar",
                "16000",
                "-ac",
                "1",
                "-c:a",
                "pcm_s16le",
                wav_file.to_str().unwrap(),
            ]);
        let status = run_local(convert, token).await?;

        if !status.status.success() {
            return Err("Gagal mengonversi audio ke WAV untuk Whisper".into());
        }

        let threads = (num_cpus() / 2).max(2).min(8).to_string();

        let mut recognize = Command::new(&cli_exe);
        recognize
            .args(&[
                "-m",
                model.to_str().unwrap(),
                "-f",
                wav_file.to_str().unwrap(),
                "-l",
                whisper_lang(language),
                "-ml",
                "1",
                "-sow",
                "-oj",
                "-of",
                out_base.to_str().unwrap(),
                "-np",
                "-t",
                &threads,
                "--prompt",
                &prompt_cut,
            ]);
        let whisper_res = run_local(recognize, token).await?;

        let json_file = PathBuf::from(format!("{}.json", out_base.display()));

        if !whisper_res.status.success() || !json_file.exists() {
            return Err(format!("Whisper gagal: {}", String::from_utf8_lossy(&whisper_res.stderr)));
        }

        let json_str = fs::read_to_string(&json_file).map_err(|e| e.to_string())?;
        parse_output(&json_str)
    }.await;
    let _ = fs::remove_file(&wav_file);
    let _ = fs::remove_file(out_base.with_extension("json"));
    result
}

fn parse_output(json_str: &str) -> Result<Vec<Heard>, String> {
    #[derive(Deserialize)]
    struct WOffset {
        from: f64,
        to: f64,
    }
    #[derive(Deserialize)]
    struct WSegment {
        text: String,
        offsets: WOffset,
    }
    #[derive(Deserialize)]
    struct WOutput {
        transcription: Option<Vec<WSegment>>,
    }

    let parsed: WOutput = serde_json::from_str(json_str).map_err(|e| e.to_string())?;
    if parsed.transcription.as_ref().is_none_or(|segments| segments.is_empty()) {
        return Err("Whisper tidak mengembalikan kata atau timing caption".into());
    }
    if parsed.transcription.as_ref().unwrap().iter().any(|s| !s.offsets.from.is_finite() || !s.offsets.to.is_finite() || s.offsets.from < 0.0 || s.offsets.to < s.offsets.from) {
        return Err("Timing caption dari Whisper tidak valid".into());
    }
    let segments: Vec<Heard> = parsed
        .transcription
        .unwrap_or_default()
        .into_iter()
        .map(|s| Heard {
            text: s.text,
            start: s.offsets.from / 1000.0,
            end: s.offsets.to / 1000.0,
        })
        .collect();

    let words = split_heard(&segments);
    if words.is_empty() { return Err("Whisper tidak mengenali kata dari audio".into()); }
    Ok(words)
}

pub async fn time_script(
    execution: &WhisperExecution,
    resource_dir: Option<&Path>,
    audio_path: &Path,
    words: &[String],
    language: &str,
    duration_sec: f64,
    tmp_dir: &Path,
    token: &CancellationToken,
) -> Result<Vec<WordTiming>, String> {
    let heard = recognize_audio(
        resource_dir,
        audio_path,
        language,
        &words.join(" "),
        tmp_dir,
        token,
        execution,
    )
    .await?;
    Ok(align_words(words, &heard, duration_sec))
}

pub fn whisper_ready_with(resource_dir: Option<&Path>) -> bool {
    let m_path = whisper_model_path();
    let ready = m_path.exists() && m_path.metadata().map(|m| m.len()).unwrap_or(0) == MODEL_BYTES;
    let cli = whisper_cli_path(resource_dir);
    ready && cli.exists()
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn execution_defaults_local_and_validates_ssh_profile() {
        assert!(WhisperExecution::from_settings(&json!({})).unwrap().ssh_profile.is_none());
        let settings = json!({"whisperTarget":"ssh","exportSshProfile":"instabox-render"});
        assert_eq!(WhisperExecution::from_settings(&settings).unwrap().ssh_profile.as_deref(), Some("instabox-render"));
        assert!(WhisperExecution::from_settings(&json!({"whisperTarget":"ssh","exportSshProfile":"-oBad"})).is_err());
        assert!(WhisperExecution::from_settings(&json!({"whisperTarget":"bad"})).is_err());
    }

    #[test]
    fn local_and_remote_json_use_identical_word_timing_units() {
        let heard = parse_output(r#"{"transcription":[{"text":" hello world","offsets":{"from":1000,"to":2000}}]}"#).unwrap();
        assert_eq!(heard.len(), 2);
        assert_eq!(heard[0].text, "hello");
        assert_eq!(heard[0].start, 1.0);
        assert_eq!(heard[1].end, 2.0);
        for invalid in [r#"{}"#, r#"{"transcription":[]}"#, r#"{"transcription":[{"text":"hi","offsets":{"from":2000,"to":1000}}]}"#] {
            assert!(parse_output(invalid).is_err());
        }
    }

    #[tokio::test]
    async fn canceled_local_work_does_not_start_a_process() {
        let token = CancellationToken::new();
        token.cancel();
        assert_eq!(run_local(Command::new("this-program-does-not-exist"), &token).await.unwrap_err(), "Dibatalkan");
    }
}
