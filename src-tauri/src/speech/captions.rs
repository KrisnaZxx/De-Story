use super::{
    align::WordTiming,
    whisper::{self, WhisperExecution},
};
use crate::{
    db::repo::{get_secret, get_settings},
    services::speko,
};
use rusqlite::Connection;
use std::path::Path;
use tokio_util::sync::CancellationToken;

pub enum CaptionExecution {
    Whisper(WhisperExecution),
    Speko { key: String, model: String },
}

impl CaptionExecution {
    pub fn from_settings(conn: &Connection) -> Result<Self, String> {
        let settings = get_settings(conn);
        match settings["captionEngine"].as_str().unwrap_or("whisper") {
            "whisper" => Ok(Self::Whisper(WhisperExecution::from_settings(&settings)?)),
            "speko" => {
                let key = get_secret(conn, "speko")
                    .filter(|k| !k.trim().is_empty())
                    .ok_or("Isi API key Speko di Pengaturan → Layanan API → Speko")?;
                let model = settings["spekoSttModel"]
                    .as_str()
                    .filter(|m| !m.is_empty())
                    .ok_or("Pilih model Speko STT di Pengaturan → Layanan API → Speko")?
                    .into();
                Ok(Self::Speko { key, model })
            }
            _ => Err("Mesin caption tidak valid".into()),
        }
    }
    pub fn available(&self, resource_dir: Option<&Path>) -> bool {
        match self {
            Self::Whisper(w) => w.available(resource_dir),
            Self::Speko { .. } => true,
        }
    }
    pub fn provider(&self) -> &'static str {
        match self {
            Self::Whisper(_) => "whisper",
            Self::Speko { .. } => "speko",
        }
    }
    pub fn target(&self) -> &'static str {
        match self {
            Self::Whisper(w) if w.ssh_profile.is_some() => "ssh",
            Self::Whisper(_) => "local",
            Self::Speko { .. } => "cloud",
        }
    }
    pub fn message(&self) -> &'static str {
        match self {
            Self::Whisper(w) => w.message(),
            Self::Speko { .. } => "Menyelaraskan caption lewat Speko STT",
        }
    }
}

pub async fn time_script(
    execution: &CaptionExecution,
    resources: Option<&Path>,
    audio: &Path,
    words: &[String],
    language: &str,
    total: f64,
    tmp: &Path,
    token: &CancellationToken,
) -> Result<Vec<WordTiming>, String> {
    match execution {
        CaptionExecution::Whisper(w) => {
            whisper::time_script(w, resources, audio, words, language, total, tmp, token).await
        }
        CaptionExecution::Speko { key, model } => {
            speko::time_script(
                key, model, resources, audio, words, language, total, tmp, token,
            )
            .await
        }
    }
}
