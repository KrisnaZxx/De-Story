use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use crate::paths::user_data_dir;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const WORKER: &str = include_str!("ssh_worker.py");

pub fn validate_profile(profile: &str) -> Result<(), String> {
    if profile.is_empty() || profile.len() > 64 || !profile.as_bytes()[0].is_ascii_alphanumeric()
        || !profile.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)) {
        return Err("Isi nama profil SSH, misalnya instabox-render. Profil harus tersambung lewat terminal terlebih dahulu.".into());
    }
    Ok(())
}

pub(crate) fn quote(value: &str) -> String { format!("'{}'", value.replace('\'', "'\"'\"'")) }

pub(crate) fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    let config = user_data_dir().join("ssh/config");
    if config.exists() { cmd.arg("-F").arg(config); }
    cmd.args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=15", "-o", "StrictHostKeyChecking=yes", "-o", "LogLevel=ERROR"])
        .kill_on_drop(true).stdin(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    cmd
}

pub(crate) fn ssh(profile: &str, remote_command: &str) -> Command {
    let mut cmd = command("ssh");
    cmd.args(["-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=3", profile, remote_command]);
    cmd
}

pub(crate) async fn checked(mut cmd: Command, token: &CancellationToken) -> Result<Vec<u8>, String> {
    let output = tokio::select! {
        _ = token.cancelled() => return Err("Ekspor dibatalkan".into()),
        result = cmd.output() => result.map_err(|e| format!("Gagal menjalankan SSH/SCP: {e}"))?,
    };
    if output.status.success() { return Ok(output.stdout); }
    let error = String::from_utf8_lossy(&output.stderr);
    let error: String = error.chars().take(1200).collect();
    Err(format!("SSH/SCP gagal: {error}"))
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConnection {
    pub profile: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(default)]
    pub identity_file: String,
}

pub fn connections(settings: &Value) -> Vec<SshConnection> {
    let mut values: Vec<SshConnection> = serde_json::from_value(settings["sshConnections"].clone()).unwrap_or_default();
    if values.is_empty() { if let Ok(old) = serde_json::from_value(settings["sshConnection"].clone()) { values.push(old); } }
    values
}

fn config_text(connections: &[SshConnection]) -> Result<String, String> {
    let mut text = String::new();
    let mut names = std::collections::HashSet::new();
    for connection in connections {
        validate_profile(&connection.profile)?;
        if !names.insert(&connection.profile) { return Err("Nama profil SSH harus unik".into()); }
        let host_ok = !connection.host.is_empty() && connection.host.len() <= 253
            && connection.host.as_bytes()[0].is_ascii_alphanumeric()
            && connection.host.bytes().all(|c| c.is_ascii_alphanumeric() || b".:-_".contains(&c));
        let user_ok = !connection.username.is_empty() && connection.username.len() <= 128
            && connection.username.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c));
        if !host_ok || !user_ok || connection.port == 0 { return Err("Host, username, atau port SSH tidak valid".into()); }
        if connection.identity_file.contains(['\n', '\r', '"', '\0']) { return Err("Lokasi private key tidak valid".into()); }
        let identity = if connection.identity_file.is_empty() { String::new() } else {
            format!("    IdentityFile \"{}\"\n    IdentitiesOnly yes\n", connection.identity_file.replace('\\', "/"))
        };
        text.push_str(&format!("Host {}\n    HostName {}\n    Port {}\n    User {}\n{}\n", connection.profile, connection.host, connection.port, connection.username, identity));
    }
    if let Some(p) = dirs::home_dir().map(|p| p.join(".ssh/config")).filter(|p| p.exists()) {
        text.push_str(&format!("Include \"{}\"\n", p.to_string_lossy().replace('\\', "/")));
    }
    Ok(text)
}

pub fn save_connections(connections: &[SshConnection]) -> Result<(), String> {
    let text = config_text(connections)?;
    let directory = user_data_dir().join("ssh");
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let temp = directory.join("config.new");
    std::fs::write(&temp, text).map_err(|e| e.to_string())?;
    std::fs::rename(temp, directory.join("config")).map_err(|e| e.to_string())
}

pub async fn test_connection(profile: &str) -> Result<Value, String> {
    validate_profile(profile)?;
    let script = "import json,os,pathlib,shutil,subprocess; f=shutil.which('ffmpeg'); d=shutil.disk_usage(pathlib.Path.home()); print(json.dumps({'cpus':os.cpu_count(),'freeGb':round(d.free/1073741824,2),'totalGb':round(d.total/1073741824,2),'ffmpeg':subprocess.check_output([f,'-version'],text=True).splitlines()[0] if f else None,'worker':(pathlib.Path.home()/'.local/share/de-story-ssh/worker.py').is_file()}))";
    let bytes = tokio::time::timeout(Duration::from_secs(30), checked(ssh(profile, &format!("python3 -c {}", quote(script))), &CancellationToken::new()))
        .await.map_err(|_| "VPS tidak merespons dalam 30 detik")??;
    serde_json::from_slice(&bytes).map_err(|_| "Respons pemeriksaan VPS tidak valid".into())
}

pub async fn install(profile: &str) -> Result<Value, String> {
    validate_profile(profile)?;
    let token = CancellationToken::new();
    let install = "if ! command -v ffmpeg >/dev/null 2>&1 || ! command -v python3 >/dev/null 2>&1; then sudo -n apt-get update && sudo -n apt-get install -y python3 ffmpeg; fi";
    tokio::time::timeout(Duration::from_secs(600), checked(ssh(profile, install), &token)).await
        .map_err(|_| "Instalasi VPS melewati batas 10 menit")??;
    checked(ssh(profile, "mkdir -p .local/share/de-story-ssh .cache/de-story-render && chmod 700 .local/share/de-story-ssh .cache/de-story-render"), &token).await?;
    let source = user_data_dir().join("ssh/worker.py");
    if let Some(parent) = source.parent() { std::fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
    std::fs::write(&source, WORKER).map_err(|e| e.to_string())?;
    let mut cmd = command("scp");
    cmd.arg(source).arg(format!("{profile}:.local/share/de-story-ssh/worker.py"));
    checked(cmd, &token).await?;
    test_connection(profile).await
}

fn validate_id(id: &str) -> Result<(), String> {
    if Uuid::parse_str(id).map(|v| v.to_string()).ok().as_deref() != Some(id) { return Err("ID hasil ekspor tidak valid".into()); }
    Ok(())
}

async fn manage(profile: &str, action: &str, id: Option<&str>) -> Result<Value, String> {
    validate_profile(profile)?;
    if let Some(id) = id { validate_id(id)?; }
    let text = format!("python3 .local/share/de-story-ssh/worker.py {} {}", quote(action), quote(id.unwrap_or("")));
    let bytes = tokio::time::timeout(Duration::from_secs(30), checked(ssh(profile, &text), &CancellationToken::new())).await
        .map_err(|_| "VPS tidak merespons dalam 30 detik")??;
    serde_json::from_slice(&bytes).map_err(|_| "Respons pengelolaan hasil VPS tidak valid".into())
}

pub async fn list_results(profile: &str) -> Result<Value, String> { manage(profile, "list", None).await }
pub async fn delete_result(profile: &str, id: &str) -> Result<Value, String> { manage(profile, "delete", Some(id)).await }

fn safe_filename(value: &str) -> Result<&str, String> {
    if value.is_empty() || value.len() > 240 || value.contains(['/', '\\', ':', '\0', '\r', '\n'])
        || !value.ends_with(".mp4") || value.starts_with('.') { return Err("Nama file hasil VPS tidak aman".into()); }
    Ok(value)
}

fn digest(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 1024 * 1024];
    loop {
        let size = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if size == 0 { break; }
        hasher.update(&buffer[..size]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

async fn download_file(profile: &str, id: &str, destination: &Path, meta: &Value, token: &CancellationToken) -> Result<(), String> {
    validate_id(id)?;
    let partial = destination.with_extension(format!("{}.part", Uuid::new_v4()));
    let mut cmd = command("scp");
    cmd.arg(format!("{profile}:.cache/de-story-render/{id}/result.mp4")).arg(&partial);
    let result = async {
        checked(cmd, token).await?;
        let expected = meta["sha256"].as_str().ok_or("VPS tidak mengembalikan checksum hasil")?;
        let path = partial.clone();
        let actual = tokio::task::spawn_blocking(move || digest(&path)).await.map_err(|e| e.to_string())??;
        if actual != expected { return Err("Checksum unduhan tidak cocok; video di VPS tetap disimpan".into()); }
        if token.is_cancelled() { return Err("Unduhan dibatalkan; video di VPS tetap disimpan".into()); }
        if destination.exists() { return Err("Nama file lokal sudah dipakai; video di VPS tetap disimpan".into()); }
        tokio::fs::rename(&partial, destination).await.map_err(|e| e.to_string())
    }.await;
    if result.is_err() { let _ = tokio::fs::remove_file(&partial).await; }
    result
}

pub async fn download_result(profile: &str, id: &str, folder: &Path, delete_after: bool) -> Result<String, String> {
    let meta = manage(profile, "get", Some(id)).await?;
    let filename = safe_filename(meta["filename"].as_str().ok_or("Nama hasil VPS tidak ditemukan")?)?;
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let mut destination = folder.join(filename);
    let stem = Path::new(filename).file_stem().unwrap().to_string_lossy();
    let mut index = 2;
    while destination.exists() { destination = folder.join(format!("{stem}-{index}.mp4")); index += 1; }
    download_file(profile, id, &destination, &meta, &CancellationToken::new()).await?;
    if delete_after { delete_result(profile, id).await.map_err(|e| format!("Video tersimpan di {} tetapi gagal dihapus dari VPS: {e}", destination.display()))?; }
    Ok(destination.to_string_lossy().to_string())
}

pub struct SshExport {
    profile: String,
    id: String,
    stage: PathBuf,
    inputs: HashMap<PathBuf, String>,
    commands: Vec<Value>,
    title: String,
    delete_after_download: bool,
}

impl SshExport {
    pub async fn new(profile: &str, job_id: &str, cwd: &Path, title: &str, delete_after_download: bool, token: &CancellationToken) -> Result<Self, String> {
        validate_profile(profile)?;
        validate_id(job_id)?;
        let status = test_connection(profile).await?;
        if !status["ffmpeg"].is_string() || status["worker"] != true { return Err("Pasang kebutuhan VPS di Pengaturan → Setting SSH terlebih dahulu".into()); }
        if token.is_cancelled() { return Err("Ekspor dibatalkan".into()); }
        let stage = cwd.join("ssh-upload").join(job_id);
        std::fs::create_dir_all(stage.join("inputs")).map_err(|e| e.to_string())?;
        std::fs::write(stage.join("worker.py"), WORKER).map_err(|e| e.to_string())?;
        Ok(Self { profile: profile.into(), id: job_id.into(), stage, inputs: HashMap::new(), commands: Vec::new(), title: title.into(), delete_after_download })
    }

    pub fn queue(&mut self, args: &[String], duration: f64) -> Result<(), String> {
        let mut mapped = args.to_vec();
        for index in 1..mapped.len() {
            if mapped[index - 1] == "-i" && Path::new(&args[index]).is_absolute() {
                let source = PathBuf::from(&args[index]);
                let relative = if let Some(hit) = self.inputs.get(&source) { hit.clone() } else {
                    let ext = source.extension().and_then(|s| s.to_str()).filter(|s| s.len() <= 8 && s.bytes().all(|c| c.is_ascii_alphanumeric())).unwrap_or("bin");
                    let name = format!("inputs/{:05}.{ext}", self.inputs.len());
                    let target = self.stage.join(&name);
                    if std::fs::hard_link(&source, &target).is_err() { std::fs::copy(&source, &target).map_err(|e| e.to_string())?; }
                    self.inputs.insert(source, name.clone());
                    name
                };
                mapped[index] = relative;
            }
        }
        if mapped.last().is_some_and(|s| Path::new(s).is_absolute()) { *mapped.last_mut().unwrap() = "result.mp4".into(); }
        self.commands.push(json!({"args":mapped,"duration":duration}));
        Ok(())
    }

    pub async fn prepare(&mut self, args: &[String], cwd: &Path, duration: f64, token: &CancellationToken) -> Result<Command, String> {
        self.queue(args, duration)?;
        for filename in ["graph.txt", "captions.ass"] {
            let source = cwd.join(filename);
            if source.exists() { std::fs::copy(source, self.stage.join(filename)).map_err(|e| e.to_string())?; }
        }
        if cwd.join("fonts").is_dir() {
            std::fs::create_dir_all(self.stage.join("fonts")).map_err(|e| e.to_string())?;
            for entry in std::fs::read_dir(cwd.join("fonts")).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                if entry.file_type().map_err(|e| e.to_string())?.is_file() {
                    std::fs::copy(entry.path(), self.stage.join("fonts").join(entry.file_name())).map_err(|e| e.to_string())?;
                }
            }
        }
        let filename = Path::new(args.last().ok_or("File hasil tidak ditemukan")?).file_name().and_then(|s| s.to_str()).ok_or("Nama hasil tidak valid")?;
        safe_filename(filename)?;
        let manifest = json!({"title":self.title,"filename":filename,"commands":self.commands});
        std::fs::write(self.stage.join("manifest.json"), serde_json::to_vec(&manifest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let mut upload = command("scp");
        upload.arg("-r").arg(&self.stage).arg(format!("{}:.cache/de-story-render/", self.profile));
        checked(upload, token).await?;
        Ok(ssh(&self.profile, &format!("cd {} && exec python3 worker.py run-plan", quote(&format!(".cache/de-story-render/{}", self.id)))))
    }

    pub async fn download(&self, destination: &Path, token: &CancellationToken) -> Result<(), String> {
        let meta = manage(&self.profile, "get", Some(&self.id)).await?;
        download_file(&self.profile, &self.id, destination, &meta, token).await?;
        if self.delete_after_download && !token.is_cancelled() {
            delete_result(&self.profile, &self.id).await.map_err(|e| format!("Video tersimpan di {} tetapi gagal dihapus dari VPS: {e}", destination.display()))?;
        }
        Ok(())
    }

    pub async fn cancel(&self) {
        let text = format!("python3 .local/share/de-story-ssh/worker.py cancel {}", quote(&self.id));
        let _ = tokio::time::timeout(Duration::from_secs(20), checked(ssh(&self.profile, &text), &CancellationToken::new())).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_migrate_preserve_all_hosts_and_reject_config_injection() {
        let old = json!({"sshConnection":{"profile":"old","host":"old.example","port":22,"username":"alice"}});
        let mut list = connections(&old);
        list.push(SshConnection { profile:"second".into(),host:"two.example".into(),port:2222,username:"bob".into(),identity_file:"/home/bob/.ssh/key".into() });
        let config = config_text(&list).unwrap();
        assert!(config.contains("Host old\n") && config.contains("Host second\n"));
        assert_eq!(config.matches("HostName ").count(),2);
        list[1].identity_file = "bad\nProxyCommand bad".into();
        assert!(config_text(&list).is_err());
        list[1] = list[0].clone(); assert!(config_text(&list).is_err());
    }

    #[test]
    fn profile_rejects_options_and_shell_syntax() {
        for invalid in ["", "-oProxyCommand=bad", "user@host", "host; rm", "two names", "../host"] { assert!(validate_profile(invalid).is_err()); }
        assert!(validate_profile("instabox-render").is_ok());
    }

    #[test]
    fn paths_and_remote_ids_reject_traversal() {
        for name in ["../result.mp4", "C:\\result.mp4", "/result.mp4", ".hidden.mp4", "a\n.mp4"] { assert!(safe_filename(name).is_err()); }
        assert!(safe_filename("cerita-2.mp4").is_ok());
        assert!(validate_id("../../home").is_err());
        assert!(validate_id(&Uuid::new_v4().to_string()).is_ok());
    }

    #[test]
    fn remote_arguments_preserve_quotes_and_disable_expansion() {
        assert_eq!(quote("hello world"), "'hello world'");
        assert_eq!(quote("it's $(secret)"), "'it'\"'\"'s $(secret)'");
    }
}
