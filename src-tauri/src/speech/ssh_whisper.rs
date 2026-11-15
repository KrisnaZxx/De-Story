use std::path::Path;
use std::time::Duration;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use crate::media::ssh_export::{checked, command, quote, ssh, validate_profile};
use crate::paths::user_data_dir;

const WORKER: &str = include_str!("ssh_whisper.py");
const REMOTE_WORKER: &str = ".local/share/de-story-ssh/whisper.py";

pub async fn status(profile: &str) -> Result<Value, String> {
    validate_profile(profile)?;
    let script = format!("if test -f {REMOTE_WORKER}; then python3 {REMOTE_WORKER} status; else printf '%s' '{{\"ready\":false}}'; fi");
    let bytes = tokio::time::timeout(Duration::from_secs(30), checked(ssh(profile, &script), &CancellationToken::new()))
        .await.map_err(|_| "Pemeriksaan Whisper VPS melewati batas 30 detik")??;
    serde_json::from_slice(&bytes).map_err(|_| "Respons Whisper VPS tidak valid".into())
}

pub async fn install(profile: &str) -> Result<Value, String> {
    validate_profile(profile)?;
    let token = CancellationToken::new();
    let folder = user_data_dir().join("ssh");
    tokio::fs::create_dir_all(&folder).await.map_err(|e| e.to_string())?;
    let source = folder.join("whisper.py");
    tokio::fs::write(&source, WORKER).await.map_err(|e| e.to_string())?;
    checked(ssh(profile, "mkdir -p .local/share/de-story-ssh .cache/de-story-whisper && chmod 700 .local/share/de-story-ssh .cache/de-story-whisper"), &token).await?;
    let mut upload = command("scp");
    upload.arg(&source).arg(format!("{profile}:{REMOTE_WORKER}"));
    checked(upload, &token).await?;
    tokio::time::timeout(Duration::from_secs(1800), checked(ssh(profile, &format!("python3 {REMOTE_WORKER} install")), &token))
        .await.map_err(|_| "Instalasi Whisper VPS melewati batas 30 menit; periksa koneksi lalu coba lagi")??;
    status(profile).await
}

pub async fn recognize(profile: &str, audio: &Path, language: &str, prompt: &str, tmp: &Path, token: &CancellationToken) -> Result<String, String> {
    validate_profile(profile)?;
    if token.is_cancelled() { return Err("Dibatalkan".into()); }
    let id = uuid::Uuid::new_v4().to_string();
    let stage = tmp.join(&id);
    let remote = format!(".cache/de-story-whisper/{id}");
    let result = async {
        tokio::fs::create_dir_all(&stage).await.map_err(|e| e.to_string())?;
        let target = stage.join("audio.input");
        if std::fs::hard_link(audio, &target).is_err() { tokio::fs::copy(audio, &target).await.map_err(|e| e.to_string())?; }
        tokio::fs::write(stage.join("whisper.py"), WORKER).await.map_err(|e| e.to_string())?;
        let params = json!({"language":language,"prompt":prompt});
        tokio::fs::write(stage.join("params.json"), serde_json::to_vec(&params).map_err(|e| e.to_string())?).await.map_err(|e| e.to_string())?;
        let mut upload = command("scp");
        upload.arg("-r").arg(&stage).arg(format!("{profile}:.cache/de-story-whisper/"));
        checked(upload, token).await.map_err(|e| format!("Upload audio gagal. Periksa koneksi dan pemasangan Whisper VPS di Pengaturan: {e}"))?;
        let script = format!("cd {} && chmod 700 . && exec python3 whisper.py run", quote(&remote));
        let bytes = checked(ssh(profile, &script), token).await?;
        String::from_utf8(bytes).map_err(|_| "Hasil Whisper VPS bukan UTF-8 yang valid".into())
    }.await;
    // Successful workers clean themselves; failed SSH still needs remote cancellation.
    if result.is_err() {
        let cleanup = format!("python3 {REMOTE_WORKER} clean {}", quote(&id));
        let _ = tokio::time::timeout(Duration::from_secs(20), checked(ssh(profile, &cleanup), &CancellationToken::new())).await;
    }
    let _ = tokio::fs::remove_dir_all(&stage).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::speech::whisper::{time_script, WhisperExecution};

    #[tokio::test]
    async fn canceled_remote_work_does_not_upload_audio() {
        let token = CancellationToken::new();
        token.cancel();
        assert_eq!(recognize("instabox-render", Path::new("missing"), "id", "", Path::new("missing"), &token).await.unwrap_err(), "Dibatalkan");
    }

    #[tokio::test]
    #[ignore = "requires an explicitly configured SSH test profile and public audio fixture"]
    async fn ssh_whisper_round_trip() {
        let profile = std::env::var("DE_STORY_SSH_PROFILE").expect("set DE_STORY_SSH_PROFILE");
        let audio = std::env::var("DE_STORY_TEST_AUDIO").expect("set DE_STORY_TEST_AUDIO");
        assert_eq!(status(&profile).await.unwrap()["ready"], true);
        let tmp = user_data_dir().join("tmp").join(format!("whisper-test-{}", uuid::Uuid::new_v4()));
        let execution = WhisperExecution { ssh_profile: Some(profile) };
        let words = ["And", "so", "my", "fellow", "Americans"].map(String::from);
        let started = std::time::Instant::now();
        let result = time_script(&execution, None, Path::new(&audio), &words, "en", 4.0, &tmp, &CancellationToken::new()).await;
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        let timed = result.unwrap();
        assert_eq!(timed.len(), words.len());
        assert!(timed.iter().all(|w| w.start >= 0.0 && w.end >= w.start && w.end <= 4.0));
        assert!(timed.iter().any(|w| w.end > w.start));
        println!("VPS round trip: {:.2}s", started.elapsed().as_secs_f64());
        let started = std::time::Instant::now();
        let local = WhisperExecution { ssh_profile: None };
        let result = time_script(&local, None, Path::new(&audio), &words, "en", 4.0, &tmp, &CancellationToken::new()).await;
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        let timed = result.unwrap();
        assert_eq!(timed.len(), words.len());
        println!("Local round trip: {:.2}s", started.elapsed().as_secs_f64());
    }
}
