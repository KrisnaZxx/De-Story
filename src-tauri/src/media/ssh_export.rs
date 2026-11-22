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
const TRANSFER: &str = include_str!("ssh_transfer.py");

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
    cmd.args(["-o", "ConnectTimeout=15", "-o", "StrictHostKeyChecking=yes", "-o", "LogLevel=ERROR"])
        .kill_on_drop(true).stdin(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    cmd
}

pub(crate) fn command_for(program: &str, profile: &str) -> Command {
    let mut cmd = command(program);
    let password = std::fs::read_to_string(user_data_dir().join("ssh/password-profiles.json")).ok()
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok()).is_some_and(|names| names.iter().any(|name| name == profile));
    if password {
        if let Ok(exe) = std::env::current_exe() {
            cmd.env("SSH_ASKPASS",exe).env("SSH_ASKPASS_REQUIRE","force").env("DE_STORY_SSH_ASKPASS_PROFILE",profile)
                .args(["-o","BatchMode=no","-o","NumberOfPasswordPrompts=1","-o","PubkeyAuthentication=no","-o","PreferredAuthentications=password,keyboard-interactive"]);
        }
    } else { cmd.args(["-o","BatchMode=yes"]); }
    cmd
}

pub fn askpass_main() -> Option<i32> {
    let profile = std::env::var("DE_STORY_SSH_ASKPASS_PROFILE").ok()?;
    if validate_profile(&profile).is_err() { return Some(1); }
    let prompt = std::env::args().nth(1).unwrap_or_default();
    if !password_prompt(&prompt) { return Some(1); }
    let result = (|| -> Result<(),String> {
        let connection = rusqlite::Connection::open_with_flags(crate::paths::db_path(), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e|e.to_string())?;
        connection.busy_timeout(Duration::from_secs(5)).map_err(|e|e.to_string())?;
        let secret = crate::db::repo::get_secret(&connection,&format!("ssh-password:{profile}")).ok_or("Sandi SSH tidak tersedia")?;
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        out.write_all(secret.as_bytes()).and_then(|_|out.write_all(b"\n")).and_then(|_|out.flush()).map_err(|e|e.to_string())
    })();
    Some(if result.is_ok() {0} else {1})
}

fn password_prompt(prompt: &str) -> bool {
    let lower = prompt.trim().to_ascii_lowercase();
    (lower.contains("password") || lower.contains("sandi")) && !lower.contains("passphrase") && !lower.contains("verification") && !lower.contains("one-time") && !lower.contains("otp")
}

pub(crate) fn ssh(profile: &str, remote_command: &str) -> Command {
    let mut cmd = command_for("ssh",profile);
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
    #[serde(default = "key_auth")]
    pub auth_method: String,
    #[serde(default)]
    pub password_configured: bool,
}
fn key_auth() -> String { "key".into() }

pub fn update_password(conn: &rusqlite::Connection, previous: Option<&SshConnection>, connection: &mut SshConnection, password: Option<&str>) -> Result<(),String> {
    let secret_name = format!("ssh-password:{}",connection.profile);
    let changed_host = previous.is_some_and(|old| old.host != connection.host || old.port != connection.port || old.username != connection.username);
    if connection.auth_method == "key" {
        crate::db::repo::clear_secret(conn,&secret_name)?;
        connection.password_configured = false;
        return Ok(());
    }
    if connection.auth_method != "password" { return Err("Metode autentikasi SSH tidak dikenal".into()); }
    if let Some(password) = password.filter(|s|!s.is_empty()) {
        if password.contains(['\n','\r','\0']) || password.len() > 4096 { return Err("Format sandi SSH tidak valid".into()); }
        crate::db::repo::set_secret(conn,&secret_name,password)?;
    } else if changed_host { return Err("Host, port, atau username berubah. Isi ulang sandi untuk tujuan SSH yang baru.".into()); }
    connection.password_configured = crate::db::repo::get_secret(conn,&secret_name).is_some();
    if !connection.password_configured { return Err("Isi sandi SSH untuk profil ini".into()); }
    Ok(())
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
        if !matches!(connection.auth_method.as_str(),"key"|"password") { return Err("Metode autentikasi SSH tidak dikenal".into()); }
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
    std::fs::rename(temp, directory.join("config")).map_err(|e| e.to_string())?;
    let names: Vec<&str> = connections.iter().filter(|c| c.auth_method == "password").map(|c|c.profile.as_str()).collect();
    std::fs::write(directory.join("password-profiles.json"),serde_json::to_vec(&names).map_err(|e|e.to_string())?).map_err(|e| e.to_string())
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
    let install = "if ! command -v ffmpeg >/dev/null 2>&1 || ! command -v python3 >/dev/null 2>&1; then if [ $(id -u) = 0 ]; then apt-get update && apt-get install -y python3 ffmpeg; else sudo -n apt-get update && sudo -n apt-get install -y python3 ffmpeg; fi; fi";
    tokio::time::timeout(Duration::from_secs(600), checked(ssh(profile, install), &token)).await
        .map_err(|_| "Instalasi VPS melewati batas 10 menit")??;
    checked(ssh(profile, "mkdir -p .local/share/de-story-ssh .cache/de-story-render && chmod 700 .local/share/de-story-ssh .cache/de-story-render"), &token).await?;
    let source = user_data_dir().join("ssh/worker.py");
    if let Some(parent) = source.parent() { std::fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
    std::fs::write(&source, WORKER).map_err(|e| e.to_string())?;
    let transfer = source.with_file_name("ssh_transfer.py");
    std::fs::write(&transfer, TRANSFER).map_err(|e| e.to_string())?;
    let mut cmd = command_for("scp",profile);
    cmd.arg(source).arg(transfer).arg(format!("{profile}:.local/share/de-story-ssh/"));
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
    let mut cmd = command_for("scp",profile);
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
    if meta["kind"] == "segments" { return Err("Bagian render sementara bukan video akhir".into()); }
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
    pub fn id(&self) -> &str { &self.id }
    pub async fn new(profile: &str, job_id: &str, cwd: &Path, title: &str, delete_after_download: bool, token: &CancellationToken) -> Result<Self, String> {
        validate_profile(profile)?;
        validate_id(job_id)?;
        let status = test_connection(profile).await?;
        if !status["ffmpeg"].is_string() || status["worker"] != true { return Err("Pasang kebutuhan VPS di Pengaturan → Pengaturan SSH terlebih dahulu".into()); }
        if token.is_cancelled() { return Err("Ekspor dibatalkan".into()); }
        let stage = cwd.join("ssh-upload").join(job_id);
        std::fs::create_dir_all(stage.join("inputs")).map_err(|e| e.to_string())?;
        std::fs::write(stage.join("worker.py"), WORKER).map_err(|e| e.to_string())?;
        std::fs::write(stage.join("ssh_transfer.py"), TRANSFER).map_err(|e| e.to_string())?;
        Ok(Self { profile: profile.into(), id: job_id.into(), stage, inputs: HashMap::new(), commands: Vec::new(), title: title.into(), delete_after_download })
    }

    pub fn queue(&mut self, args: &[String], duration: f64) -> Result<(), String> {
        let mut mapped = args.to_vec();
        for index in 1..mapped.len() {
            if (mapped[index - 1] == "-i" || mapped[index - 1] == "-filter_complex_script") && Path::new(&args[index]).is_absolute() {
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
        let filename = Path::new(args.last().ok_or("File hasil tidak ditemukan")?).file_name().and_then(|s| s.to_str()).ok_or("Nama hasil tidak valid")?;
        safe_filename(filename)?;
        self.upload(cwd, filename, &[], token).await
    }

    async fn upload(&self, cwd: &Path, filename: &str, segments: &[String], token: &CancellationToken) -> Result<Command, String> {
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
        let manifest = json!({"title":self.title,"filename":filename,"commands":self.commands,"segments":segments});
        std::fs::write(self.stage.join("manifest.json"), serde_json::to_vec(&manifest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let mut upload = command_for("scp",&self.profile);
        upload.arg("-r").arg(&self.stage).arg(format!("{}:.cache/de-story-render/", self.profile));
        checked(upload, token).await?;
        Ok(self.worker_command("start"))
    }

    fn worker_command(&self, action: &str) -> Command {
        ssh(&self.profile, &format!("cd {} && exec python3 worker.py {action} {}",
            quote(&format!(".cache/de-story-render/{}", self.id)), quote(&self.id)))
    }

    pub async fn run<F: FnMut(f64)>(&mut self, args: &[String], cwd: &Path, duration: f64,
        token: &CancellationToken, progress: F) -> Result<(), String> {
        let start = self.prepare(args, cwd, duration, token).await?;
        self.wait(start, token, progress).await?;
        self.download(Path::new(args.last().ok_or("File hasil tidak ditemukan")?), token).await
    }

    async fn wait<F: FnMut(f64)>(&self, command: Command, token: &CancellationToken, mut progress: F) -> Result<Value, String> {
        let mut start = Some(command);
        let mut disconnected = None;
        loop {
            if token.is_cancelled() { self.cancel().await; return Err("Dibatalkan".into()); }
            let action = if start.is_some() { "start" } else { "status" };
            let command = start.take().unwrap_or_else(|| self.worker_command(action));
            let response = tokio::time::timeout(Duration::from_secs(35), checked(command, token)).await;
            let status = match response {
                Ok(Ok(bytes)) => serde_json::from_slice::<Value>(&bytes).map_err(|_| "Status render VPS tidak valid".to_string()),
                Ok(Err(error)) => Err(error),
                Err(_) => Err("Koneksi VPS melewati batas waktu".into()),
            };
            match status {
                Ok(meta) => {
                    disconnected = None;
                    if let Some(value) = meta["progress"].as_f64() { progress(value.clamp(0.0, 1.0)); }
                    match meta["state"].as_str() {
                        Some("completed") => return Ok(meta),
                        Some("failed") => return Err(format!("Render VPS gagal: {}", meta["error"].as_str().unwrap_or("Proses render berhenti di VPS"))),
                        Some("queued" | "running") => {},
                        _ => return Err("Status render VPS tidak dikenali; periksa hasil video di pengaturan SSH".into()),
                    }
                },
                Err(_) if token.is_cancelled() => { self.cancel().await; return Err("Dibatalkan".into()); },
                Err(_) => {
                    // Retry the idempotent start when SSH lost its acknowledgement.
                    if action == "start" { start = Some(self.worker_command("start")); }
                    let since = disconnected.get_or_insert_with(std::time::Instant::now);
                    if since.elapsed() > Duration::from_secs(600) {
                        return Err("Koneksi VPS terputus selama 10 menit. Render tetap berjalan di VPS; periksa hasil video di Pengaturan SSH untuk mengunduhnya.".into());
                    }
                }
            }
            tokio::select! {
                _ = token.cancelled() => {},
                _ = tokio::time::sleep(Duration::from_secs(3)) => {},
            }
        }
    }

    pub async fn prepare_receiver(&self, token: &CancellationToken) -> Result<(), String> {
        let mut upload = command_for("scp", &self.profile);
        upload.arg("-r").arg(&self.stage).arg(format!("{}:.cache/de-story-render/", self.profile));
        checked(upload, token).await?;
        Ok(())
    }

    pub async fn require_direct(&self, primary: &Self, token: &CancellationToken) -> Result<(),String> {
        if !self.profile.eq_ignore_ascii_case(&primary.profile) {
            checked(ssh(&self.profile,"command -v instabox-port >/dev/null"),token).await
                .map_err(|_|format!("VPS {} memerlukan instabox-port untuk transfer langsung. Server lain tetap dapat memakai ekspor satu VPS.",self.profile))?;
        }
        Ok(())
    }

    pub async fn run_segments<F: FnMut(f64)>(&self, files: &[String], cwd: &Path, primary: &Self, token: &CancellationToken, mut progress: F) -> Result<(), String> {
        for name in files { validate_segment_name(name)?; }
        if files.is_empty() { return Err("Tidak ada klip untuk VPS ini".into()); }
        let start = self.upload(cwd, "segments", files, token).await?;
        let meta = self.wait(start, token, &mut progress).await?;
        let received = meta["files"].as_array().filter(|items| items.len() == files.len()).ok_or("Daftar klip hasil VPS tidak lengkap")?;
        for (expected, item) in files.iter().zip(received) {
            if item["name"].as_str() != Some(expected.as_str()) || !item["sha256"].as_str().is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())) {
                return Err("Metadata klip hasil VPS tidak valid".into());
            }
        }
        let local = self.profile.eq_ignore_ascii_case(&primary.profile);
        let transfer = async {
            let ticket = if local { json!({"localJob":self.id,"files":received}) } else {
                let bytes = checked(self.worker_command("start-transfer"), token).await?;
                let ticket: Value = serde_json::from_slice(&bytes).map_err(|_|"Respons transfer langsung tidak valid")?;
                if ticket["files"].as_array() != Some(received) { return Err("Daftar transfer langsung berbeda dari hasil render".into()); }
                ticket
            };
            let filename = format!("transfer-{}.json", self.id);
            let path = self.stage.join(&filename);
            std::fs::write(&path, serde_json::to_vec(&ticket).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            let mut scp = command_for("scp", &primary.profile);
            scp.arg(&path).arg(format!("{}:.cache/de-story-render/{}/{}", primary.profile, primary.id, filename));
            let uploaded = checked(scp, token).await;
            let _ = std::fs::remove_file(&path);
            uploaded?;
            let command = ssh(&primary.profile, &format!("cd {} && exec python3 worker.py receive-transfer {} {}",
                quote(&format!(".cache/de-story-render/{}", primary.id)), quote(&primary.id), quote(&filename)));
            let bytes = checked(command, token).await?;
            let receipt: Value = serde_json::from_slice(&bytes).map_err(|_|"Konfirmasi transfer VPS tidak valid")?;
            if receipt["files"].as_array() != Some(received) { return Err("Konfirmasi checksum VPS utama tidak cocok".into()); }
            Ok::<(),String>(())
        }.await;
        if !local {
            let stopped = tokio::time::timeout(Duration::from_secs(35), checked(self.worker_command("stop-transfer"), &CancellationToken::new())).await;
            if transfer.is_ok() && !matches!(stopped,Ok(Ok(_))) { return Err("Bagian telah diterima, tetapi port privat belum dapat ditutup; periksa Ports di InstaBox. Hasil sumber tetap disimpan.".into()); }
        }
        transfer?;
        delete_result(&self.profile, &self.id).await.map_err(|e|format!("Bagian sudah diterima VPS utama tetapi belum terhapus dari VPS {}: {e}", self.profile))?;
        Ok(())
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
        let _ = tokio::time::timeout(Duration::from_secs(20), checked(self.worker_command("cancel"), &CancellationToken::new())).await;
    }
}

fn validate_segment_name(name: &str) -> Result<(), String> {
    let number = name.strip_prefix("seg_").and_then(|v|v.strip_suffix(".mp4")).ok_or("Nama klip sementara tidak valid")?;
    if !(3..=6).contains(&number.len()) || !number.bytes().all(|c|c.is_ascii_digit()) { return Err("Nama klip sementara tidak valid".into()); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "writes a DPAPI-encrypted fixture for the loopback SSH password test"]
    fn write_askpass_fixture() {
        let directory = PathBuf::from(std::env::var_os("DE_STORY_SSH_FIXTURE_DIR").expect("fixture path"));
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(".toolchain").canonicalize().unwrap();
        assert_eq!(directory.parent().unwrap().canonicalize().unwrap(),workspace);
        assert!(directory.file_name().unwrap().to_string_lossy().starts_with("ssh-password-check-"));
        std::fs::create_dir_all(&directory).unwrap();
        let conn = rusqlite::Connection::open(directory.join("studio.db")).unwrap();
        for migration in crate::db::MIGRATIONS { conn.execute_batch(migration).unwrap(); }
        crate::db::repo::set_secret(&conn,"ssh-password:fixture-password","fixture password only").unwrap();
    }

    #[test]
    fn password_is_encrypted_retained_only_for_same_host_and_removed_on_key_auth() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        for migration in crate::db::MIGRATIONS { conn.execute_batch(migration).unwrap(); }
        let mut connection = SshConnection {profile:"test-password".into(),host:"example.test".into(),port:22,username:"demo".into(),identity_file:"".into(),auth_method:"password".into(),password_configured:false};
        update_password(&conn,None,&mut connection,Some("test password ; $(not-a-command)")).unwrap();
        let bytes: Vec<u8> = conn.query_row("SELECT value FROM secrets WHERE provider='ssh-password:test-password'",[],|r|r.get(0)).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("test password"));
        assert!(connection.password_configured);
        let previous = connection.clone();
        update_password(&conn,Some(&previous),&mut connection,None).unwrap();
        connection.host="different.test".into();
        assert!(update_password(&conn,Some(&previous),&mut connection,None).is_err());
        connection.auth_method="key".into();
        update_password(&conn,Some(&previous),&mut connection,None).unwrap();
        assert!(!connection.password_configured);
        assert!(crate::db::repo::get_secret(&conn,"ssh-password:test-password").is_none());
        assert!(!config_text(&[connection]).unwrap().contains("test password"));
    }

    #[test]
    fn profiles_migrate_preserve_all_hosts_and_reject_config_injection() {
        let old = json!({"sshConnection":{"profile":"old","host":"old.example","port":22,"username":"alice"}});
        let mut list = connections(&old);
        list.push(SshConnection { profile:"second".into(),host:"two.example".into(),port:2222,username:"bob".into(),identity_file:"/home/bob/.ssh/key".into(),auth_method:"key".into(),password_configured:false });
        let config = config_text(&list).unwrap();
        assert!(config.contains("Host old\n") && config.contains("Host second\n"));
        assert_eq!(config.matches("HostName ").count(),2);
        list[1].identity_file = "bad\nProxyCommand bad".into();
        assert!(config_text(&list).is_err());
        list[1] = list[0].clone(); assert!(config_text(&list).is_err());
    }

    #[test]
    fn profile_rejects_options_and_shell_syntax() {
        assert!(password_prompt("alice@example password:"));
        assert!(!password_prompt("Enter passphrase for key:"));
        assert!(!password_prompt("OTP verification code:"));
        assert!(!password_prompt("Are you sure you want to continue connecting?"));
        for invalid in ["", "-oProxyCommand=bad", "user@host", "host; rm", "two names", "../host"] { assert!(validate_profile(invalid).is_err()); }
        assert!(validate_profile("instabox-render").is_ok());
    }

    #[test]
    fn paths_and_remote_ids_reject_traversal() {
        for name in ["../seg_000.mp4", "seg_0.mp4", "seg_1234567.mp4", "seg_000.mp4/extra", "seg_00a.mp4"] { assert!(validate_segment_name(name).is_err()); }
        for name in ["seg_000.mp4", "seg_1000.mp4", "seg_123456.mp4"] { assert!(validate_segment_name(name).is_ok()); }
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
