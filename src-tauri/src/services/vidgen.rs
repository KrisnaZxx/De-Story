use crate::db::{
    repo::{
        get_secret, get_settings, patch_settings, read_cache, set_secret, update_job, write_cache,
    },
    DbPool,
};
use crate::jobs::TaskCtx;
use reqwest::{Client, Response, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use sha2::{Digest, Sha256};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex as AsyncMutex, Notify};
use tokio_util::sync::CancellationToken;

const BASE: &str = "https://vidgen.id";
pub const MODEL: &str = "gemini-omni-1.2-unlimited";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub label: String,
    pub enabled: bool,
}

fn accounts(db: &DbPool) -> Result<Vec<Account>, String> {
    let conn = db.lock().unwrap();
    serde_json::from_value(get_settings(&conn)["vidgenAccounts"].clone())
        .map_err(|_| "Daftar akun Vidgen tidak valid".into())
}

fn secret_name(id: &str) -> Result<String, String> {
    if uuid::Uuid::parse_str(id)
        .map(|u| u.to_string())
        .ok()
        .as_deref()
        != Some(id)
    {
        return Err("ID akun Vidgen tidak valid".into());
    }
    Ok(format!("vidgen:{id}"))
}

pub fn list_accounts(db: &DbPool) -> Result<Vec<Value>, String> {
    let list = accounts(db)?;
    let conn = db.lock().unwrap();
    list.into_iter().map(|a| {
        let secret = secret_name(&a.id)?;
        let configured = get_secret(&conn, &secret).is_some();
        let info = read_cache(&conn, &secret).map(|(v,_)| v);
        Ok(json!({"id":a.id,"label":a.label,"enabled":a.enabled,"configured":configured,"info":info}))
    }).collect()
}

pub fn save_account(
    db: &DbPool,
    id: Option<String>,
    label: String,
    key: String,
    enabled: bool,
) -> Result<Vec<Value>, String> {
    let id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let secret = secret_name(&id)?;
    let label = label.trim();
    if label.is_empty() || label.chars().count() > 80 {
        return Err("Isi nama akun Vidgen (maksimal 80 karakter)".into());
    }
    let key = key.trim();
    if !key.is_empty()
        && (!key.starts_with("vg_")
            || key.len() < 8
            || key.len() > 512
            || key.chars().any(char::is_whitespace)
            || key.chars().any(char::is_control))
    {
        return Err("Format API key Vidgen tidak valid; gunakan key vg_… dari halaman Akun".into());
    }
    {
        let conn = db.lock().unwrap();
        let transaction = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let mut list: Vec<Account> =
            serde_json::from_value(get_settings(&transaction)["vidgenAccounts"].clone())
                .map_err(|_| "Daftar akun Vidgen tidak valid")?;
        if key.is_empty() && get_secret(&transaction, &secret).is_none() {
            return Err("Masukkan API key untuk akun ini".into());
        }
        if !key.is_empty() {
            set_secret(&transaction, &secret, key)?;
            crate::db::repo::drop_cache(&transaction, &secret)?;
        }
        let account = Account {
            id: id.clone(),
            label: label.into(),
            enabled,
        };
        if let Some(old) = list.iter_mut().find(|a| a.id == id) {
            *old = account;
        } else {
            list.push(account);
        }
        patch_settings(&transaction, json!({"vidgenAccounts":list}))?;
        transaction.commit().map_err(|e| e.to_string())?;
    }
    list_accounts(db)
}

pub fn remove_account(db: &DbPool, id: &str) -> Result<Vec<Value>, String> {
    let secret = secret_name(id)?;
    {
        let conn = db.lock().unwrap();
        let transaction = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let mut list: Vec<Account> =
            serde_json::from_value(get_settings(&transaction)["vidgenAccounts"].clone())
                .map_err(|_| "Daftar akun Vidgen tidak valid")?;
        list.retain(|a| a.id != id);
        crate::db::repo::clear_secret(&transaction, &secret)?;
        crate::db::repo::drop_cache(&transaction, &secret)?;
        patch_settings(&transaction, json!({"vidgenAccounts":list}))?;
        transaction.commit().map_err(|e| e.to_string())?;
    }
    list_accounts(db)
}

#[derive(Debug)]
struct ApiError {
    code: u16,
    message: String,
    retry_after: u64,
}
impl ApiError {
    fn text(message: impl Into<String>) -> Self {
        Self {
            code: 0,
            message: message.into(),
            retry_after: 30,
        }
    }
}

#[derive(Clone)]
struct Api {
    client: Client,
    base: String,
}
impl Api {
    fn new() -> Self {
        Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(180))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("HTTP client"),
            base: BASE.into(),
        }
    }
    async fn decode(&self, response: Response, key: &str) -> Result<Value, ApiError> {
        let code = response.status();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);
        let value: Value = response.json().await.map_err(|_| ApiError {
            code: code.as_u16(),
            message: "Respons Vidgen bukan JSON yang valid".into(),
            retry_after,
        })?;
        if !code.is_success() || value["success"] == false {
            let message = value["error"]
                .as_str()
                .unwrap_or("Permintaan Vidgen ditolak")
                .to_string();
            let message = if key.is_empty() {
                message
            } else {
                message.replace(key, "[key]")
            };
            return Err(ApiError {
                code: code.as_u16(),
                message,
                retry_after,
            });
        }
        Ok(value)
    }
    async fn get(&self, path: &str, key: &str) -> Result<Value, ApiError> {
        let request = self
            .client
            .get(format!("{}{}", self.base, path))
            .timeout(Duration::from_secs(30));
        let response = if key.is_empty() {
            request
        } else {
            request.bearer_auth(key)
        }
        .send()
        .await
        .map_err(|_| ApiError::text("Tidak dapat terhubung ke Vidgen"))?;
        self.decode(response, key).await
    }
    async fn upload(&self, key: &str, path: &Path, model: &str) -> Result<Value, ApiError> {
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|_| ApiError::text("Gambar klip tidak bisa dibaca"))?;
        if bytes.len() > 32 * 1024 * 1024 {
            return Err(ApiError::text(
                "Gambar referensi terlalu besar (maksimal 32 MB)",
            ));
        }
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(
                "reference".to_string()
                    + path
                        .extension()
                        .and_then(|s| s.to_str())
                        .map(|s| format!(".{s}"))
                        .as_deref()
                        .unwrap_or(".png"),
            )
            .mime_str(mime.as_ref())
            .map_err(|_| ApiError::text("Jenis gambar tidak valid"))?;
        let form = reqwest::multipart::Form::new()
            .part("file", part)
            .text("model", model.to_string());
        let response = self
            .client
            .post(format!("{}/api/v1/upload", self.base))
            .bearer_auth(key)
            .multipart(form)
            .send()
            .await
            .map_err(|_| ApiError::text("Upload gambar ke Vidgen gagal"))?;
        let value = self.decode(response, key).await?;
        if value["asset"]["url"].as_str().is_none() {
            return Err(ApiError::text(
                "Vidgen tidak mengembalikan referensi gambar",
            ));
        }
        Ok(value["asset"].clone())
    }
    async fn submit(
        &self,
        key: &str,
        body: &Value,
        token: &CancellationToken,
    ) -> Result<Value, ApiError> {
        for attempt in 0..5 {
            let result = self
                .client
                .post(format!("{}/api/v1/generate", self.base))
                .bearer_auth(key)
                .json(body)
                .send()
                .await;
            let result = match result {
                Ok(response) => self.decode(response, key).await,
                Err(_) => Err(ApiError::text("Respons submit Vidgen terputus")),
            };
            match result {
                Ok(v) => return Ok(v),
                Err(e) if attempt < 4 && (e.code == 0 || e.code >= 500) => {
                    pause(token, Duration::from_secs(2_u64.pow(attempt + 1)))
                        .await
                        .map_err(ApiError::text)?;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }
    async fn download(
        &self,
        key: &str,
        value: &str,
        token: &CancellationToken,
        video: bool,
    ) -> Result<Vec<u8>, String> {
        let base = Url::parse(&self.base).map_err(|_| "Alamat Vidgen tidak valid")?;
        let mut url = base
            .join(value)
            .map_err(|_| "URL hasil Vidgen tidak valid")?;
        for _ in 0..5 {
            if url.scheme() != "https" && url.origin() != base.origin() {
                return Err("Hasil Vidgen harus memakai HTTPS".into());
            }
            if !url.username().is_empty() || url.password().is_some() {
                return Err("URL hasil Vidgen tidak aman".into());
            }
            let mut request = self.client.get(url.clone());
            if url.origin() == base.origin() {
                request = request.bearer_auth(key);
            }
            let mut response = tokio::select! { _ = token.cancelled() => return Err("Dibatalkan".into()), result=request.send()=>result.map_err(|_| "Unduhan Vidgen gagal")? };
            if response.status().is_redirection() {
                let next = response
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or("Redirect hasil Vidgen tidak valid")?;
                url = url
                    .join(next)
                    .map_err(|_| "Redirect hasil Vidgen tidak valid")?;
                continue;
            }
            if !response.status().is_success() {
                return Err(format!(
                    "Unduhan Vidgen ditolak (HTTP {})",
                    response.status()
                ));
            }
            let mut bytes = Vec::new();
            loop {
                let chunk = tokio::select! { _=token.cancelled()=>return Err("Dibatalkan".into()), result=response.chunk()=>result.map_err(|_| "Unduhan Vidgen terputus")? };
                let Some(chunk) = chunk else {
                    break;
                };
                if bytes.len() + chunk.len() > if video { 512 * 1024 * 1024 } else { 32 * 1024 * 1024 } {
                    return Err("Video Vidgen melebihi 512 MB".into());
                }
                bytes.extend_from_slice(&chunk);
            }
            let valid = if video { bytes.get(4..8) == Some(b"ftyp") } else { bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.starts_with(&[0xff,0xd8,0xff]) || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP")) };
            if !valid { return Err("Format hasil Vidgen tidak sesuai jenis media".into()); }
            return Ok(bytes);
        }
        Err("Terlalu banyak redirect saat mengunduh video Vidgen".into())
    }
}

async fn pause(token: &CancellationToken, duration: Duration) -> Result<(), String> {
    tokio::select! { _=token.cancelled()=>Err("Dibatalkan; job yang sudah dikirim tetap dapat selesai di Vidgen".into()), _=tokio::time::sleep(duration)=>Ok(()) }
}

fn public_info(value: &Value) -> Value {
    json!({"ok":true,"checkedAt":chrono::Utc::now().timestamp_millis(),
        "user":{"uid":value["user"]["uid"],"email":value["user"]["email"],"credits":value["user"]["credits"]},
        "unlimited":{"active":value["unlimited"]["active"],"daysLeft":value["unlimited"]["daysLeft"],"expiresAt":value["unlimited"]["expiresAt"]},
        "limits":{"dailyVideosRemaining":value["limits"]["dailyVideosRemaining"],"currentActiveJobs":value["limits"]["currentActiveJobs"],"maxConcurrentJobs":value["limits"]["maxConcurrentJobs"]}})
}

pub async fn check_account(db: &DbPool, id: &str) -> Result<Vec<Value>, String> {
    let secret = secret_name(id)?;
    let key = {
        let conn = db.lock().unwrap();
        get_secret(&conn, &secret).ok_or("API key Vidgen belum diisi atau tidak bisa dibuka")?
    };
    let result = Api::new().get("/api/v1/user/credits", &key).await;
    let info = match result {
        Ok(v) => public_info(&v),
        Err(e) => {
            json!({"ok":false,"error":e.message,"checkedAt":chrono::Utc::now().timestamp_millis()})
        }
    };
    {
        let conn = db.lock().unwrap();
        if get_secret(&conn, &secret).as_deref() == Some(key.as_str()) {
            write_cache(&conn, &secret, &info)?;
        }
    }
    list_accounts(db)
}

#[derive(Default)]
struct Slots {
    users: HashMap<String, String>,
    next: usize,
}
impl Slots {
    fn reserve(&mut self, uid: &str, id: &str, max: usize) -> bool {
        if self.users.contains_key(uid) || (max > 0 && self.users.len() >= max) {
            return false;
        }
        self.users.insert(uid.into(), id.into());
        self.next = self.next.wrapping_add(1);
        true
    }
}
struct Pool {
    slots: Mutex<Slots>,
    select: AsyncMutex<()>,
    changed: Notify,
}
fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| Pool {
        slots: Mutex::new(Slots::default()),
        select: AsyncMutex::new(()),
        changed: Notify::new(),
    })
}
struct Lease {
    account: Account,
    uid: String,
    key: String,
}
impl Drop for Lease {
    fn drop(&mut self) {
        pool().slots.lock().unwrap().users.remove(&self.uid);
        pool().changed.notify_waiters();
    }
}

fn available_uid(info: &Value) -> Result<Option<&str>, &'static str> {
    if info["unlimited"]["active"] != true {
        return Err("paket Unlimited tidak aktif");
    }
    let remaining = info["limits"]["dailyVideosRemaining"]
        .as_u64()
        .ok_or("respons kuota akun tidak lengkap")?;
    let active = info["limits"]["currentActiveJobs"]
        .as_u64()
        .ok_or("respons batas akun tidak lengkap")?;
    if remaining == 0 {
        return Err("kuota video harian habis (reset 00.00 WIB)");
    }
    let uid = info["user"]["uid"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("Vidgen tidak mengembalikan identitas akun")?;
    Ok(if active > 0 { None } else { Some(uid) })
}

async fn acquire(db: &DbPool, api: &Api, ctx: &TaskCtx, image_cost: Option<f64>) -> Result<Lease, String> {
    let _selection = tokio::select! { _=ctx.token.cancelled()=>return Err("Dibatalkan".into()), guard=pool().select.lock()=>guard };
    let deadline = Instant::now() + Duration::from_secs(7200);
    loop {
        if ctx.token.is_cancelled() {
            return Err("Dibatalkan".into());
        }
        let changed = pool().changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        if Instant::now() > deadline {
            return Err("Antrean Vidgen melewati batas dua jam".into());
        }
        let mut list = accounts(db)?;
        list.retain(|a| a.enabled);
        if list.is_empty() {
            return Err(
                "Tambahkan dan aktifkan akun Vidgen di Pengaturan → Layanan AI → Vidgen".into(),
            );
        }
        let start = pool().slots.lock().unwrap().next % list.len();
        list.rotate_left(start);
        let max = {
            let conn = db.lock().unwrap();
            get_settings(&conn)["vidgenParallelAccounts"]
                .as_u64()
                .unwrap_or(0) as usize
        };
        let mut waiting = false;
        let mut errors = Vec::new();
        for account in list {
            if {
                let slots = pool().slots.lock().unwrap();
                slots.users.values().any(|id| id == &account.id)
                    || (max > 0 && slots.users.len() >= max)
            } {
                waiting = true;
                continue;
            }
            ctx.progress(
                0.07,
                Some(&format!("Memeriksa akun Vidgen {}", account.label)),
            );
            let secret = secret_name(&account.id)?;
            let key = {
                let conn = db.lock().unwrap();
                get_secret(&conn, &secret)
            };
            let Some(key) = key else {
                errors.push(format!("{}: API key tidak tersedia", account.label));
                continue;
            };
            let result = tokio::select! {_=ctx.token.cancelled()=>return Err("Dibatalkan".into()), r=api.get("/api/v1/user/credits",&key)=>r};
            let info = match result {
                Ok(v) => v,
                Err(e) => {
                    errors.push(format!("{}: {}", account.label, e.message));
                    continue;
                }
            };
            {
                let conn = db.lock().unwrap();
                if get_secret(&conn, &secret).as_deref() == Some(key.as_str()) {
                    write_cache(&conn, &secret, &public_info(&info))?;
                }
            }
            let uid = match if let Some(cost) = image_cost { available_image_uid(&info, cost) } else { available_uid(&info) } {
                Ok(Some(uid)) => uid.to_string(),
                Ok(None) => {
                    waiting = true;
                    continue;
                }
                Err(error) => {
                    errors.push(format!("{}: {error}", account.label));
                    continue;
                }
            };
            if pool().slots.lock().unwrap().reserve(&uid, &account.id, max) {
                return Ok(Lease { account, uid, key });
            }
            waiting = true;
        }
        if !waiting {
            return Err(format!(
                "Tidak ada akun Vidgen yang siap. {}",
                errors.join("; ")
            ));
        }
        ctx.progress(0.08, Some("Menunggu akun Vidgen yang tersedia"));
        tokio::select! {_=ctx.token.cancelled()=>return Err("Dibatalkan".into()),_=changed=>{},_=tokio::time::sleep(Duration::from_secs(15))=>{}}
    }
}

fn parameters(catalog: &Value, ratio: &str, resolution: Option<&str>) -> Result<Value, String> {
    let model = catalog["models"]["video"]
        .as_array()
        .and_then(|a| a.iter().find(|m| m["id"] == MODEL))
        .ok_or("Omni 1.2 Unlimited sedang tidak tersedia di katalog Vidgen")?;
    if model["unlimitedOnly"] != true || model["creditsDefault"].as_f64() != Some(0.0) {
        return Err("Ketentuan biaya model Vidgen berubah. Generasi dihentikan agar tidak memakai kredit berbayar.".into());
    }
    let caps = &model["capabilities"];
    if caps["maxReferenceImages"].as_u64().unwrap_or(0) < 1 {
        return Err("Model Vidgen tidak menerima gambar referensi".into());
    }
    let has = |field: &str, want: &str| {
        caps[field]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(want)))
    };
    if !has("ratios", ratio) || !has("durations", "15") {
        return Err(
            "Durasi atau rasio model Vidgen berubah; periksa katalog sebelum mencoba lagi".into(),
        );
    }
    let resolution = resolution.unwrap_or("1080p");
    if !has("resolutions", resolution) {
        return Err("Resolusi tidak tersedia pada model Vidgen".into());
    }
    Ok(json!({"duration":"15","ratio":ratio,"resolution":resolution}))
}

pub async fn generate_video(db: &DbPool, ctx: &TaskCtx, image: &Path, prompt: &str, ratio: &str, resolution: Option<&str>) -> Result<(Vec<u8>, Value), String> {
    generate_media(db, ctx, MODEL, &[image.to_path_buf()], prompt, ratio, resolution, true).await
}

pub async fn generate_image(db: &DbPool, ctx: &TaskCtx, model: &str, references: &[Vec<u8>], prompt: &str, ratio: &str) -> Result<(Vec<u8>, Value), String> {
    let directory = crate::paths::user_data_dir().join("tmp").join(format!("vidgen-{}", ctx.job_id));
    tokio::fs::create_dir_all(&directory).await.map_err(|e| e.to_string())?;
    let result = async {
        let mut paths = Vec::new();
        for (i, bytes) in references.iter().enumerate() {
            let ext = if bytes.starts_with(&[0xff,0xd8,0xff]) { "jpg" } else if bytes.starts_with(b"RIFF") { "webp" } else { "png" };
            let path = directory.join(format!("{i}.{ext}"));
            tokio::fs::write(&path, bytes).await.map_err(|e| e.to_string())?;
            paths.push(path);
        }
        generate_media(db, ctx, model.strip_prefix("vidgen:").ok_or("ID model Vidgen tidak valid")?, &paths, prompt, ratio, None, false).await
    }.await;
    let _ = tokio::fs::remove_dir_all(directory).await;
    result
}

fn available_image_uid(info: &Value, cost: f64) -> Result<Option<&str>, &'static str> {
    if info["user"]["credits"].as_f64().filter(|n| n.is_finite() && *n >= cost).is_none() { return Err("kredit gambar Vidgen tidak cukup"); }
    let active = info["limits"]["currentActiveJobs"].as_u64().ok_or("respons batas akun tidak lengkap")?;
    let uid = info["user"]["uid"].as_str().filter(|s| !s.is_empty()).ok_or("identitas akun tidak tersedia")?;
    Ok(if active > 0 { None } else { Some(uid) })
}

fn image_parameters(model: &Value, ratio: &str, refs: usize) -> Result<(Value,f64), String> {
    let c = &model["capabilities"];
    if model["unlimitedOnly"] == true { return Err("Model gambar Unlimited belum didukung; pilih model kredit Vidgen".into()); }
    if !c["ratios"].as_array().is_some_and(|a| a.iter().any(|v| v == ratio)) || refs > c["maxReferenceImages"].as_u64().unwrap_or(0) as usize { return Err("Rasio atau jumlah referensi tidak didukung model Vidgen".into()); }
    let resolutions = c["resolutions"].as_array().ok_or("Katalog resolusi gambar tidak lengkap")?;
    let resolution = resolutions.iter().find(|v| *v == "2K").or_else(|| resolutions.first()).and_then(Value::as_str).ok_or("Resolusi gambar kosong")?;
    let mut params = json!({"ratio":ratio,"resolution":resolution});
    let quality = c["qualities"].as_array().and_then(|a| a.iter().find(|v| *v == "high").or_else(|| a.first())).and_then(Value::as_str);
    if let Some(q) = quality { params["quality"] = json!(q); }
    if c["outputFormats"].as_array().is_some_and(|a| a.iter().any(|v| v == "png")) { params["output_format"] = json!("png"); }
    let price = quality.and_then(|q| model["priceMatrix"][format!("{q}:{resolution}")].as_f64()).or_else(|| model["priceMatrix"][resolution].as_f64()).or_else(|| model["creditsDefault"].as_f64()).filter(|n| n.is_finite() && *n >= 0.0).ok_or("Harga gambar Vidgen tidak tersedia")?;
    Ok((params,price))
}

pub fn image_estimate(model: &str, ratio: &str) -> Option<f64> {
    let cat: Value = serde_json::from_str(include_str!("../../../src/shared/vidgen-catalog.json")).ok()?;
    let id = model.strip_prefix("vidgen:")?;
    let model = cat["imagePricing"].as_array()?.iter().find(|m| m["id"] == id)?;
    image_parameters(model, ratio, 0).ok().map(|(_,cost)| cost)
}

async fn generate_media(db: &DbPool, ctx: &TaskCtx, model_id: &str, images: &[PathBuf], prompt: &str, ratio: &str, resolution: Option<&str>, video: bool) -> Result<(Vec<u8>, Value), String> {
    let api = Api::new();
    let catalog = {
        let _guard = tokio::select! {_=ctx.token.cancelled()=>return Err("Dibatalkan".into()),guard=pool().select.lock()=>guard};
        let cached = {
            let conn = db.lock().unwrap();
            read_cache(&conn, "vidgen:catalog")
                .filter(|(_, at)| chrono::Utc::now().timestamp_millis() - at < 120_000)
        };
        if let Some((value, _)) = cached {
            value
        } else {
            let value = tokio::select! {_=ctx.token.cancelled()=>return Err("Dibatalkan".into()),r=api.get("/api/v1/models","")=>r.map_err(|e|e.message)?};
            {
                let conn = db.lock().unwrap();
                write_cache(&conn, "vidgen:catalog", &value)?;
            }
            value
        }
    };
    let (params, cost) = if video { (parameters(&catalog, ratio, resolution)?, 0.0) } else {
        let model = catalog["models"]["image"].as_array().and_then(|a| a.iter().find(|m| m["id"] == model_id)).ok_or("Model gambar tidak tersedia di Vidgen")?;
        let (params,cost) = image_parameters(model, ratio, images.len())?;
        let estimate = image_estimate(&format!("vidgen:{model_id}"), ratio).ok_or("Perkiraan harga model belum tersedia")?;
        if cost > estimate + 0.0001 { return Err("Harga gambar Vidgen naik dari katalog aplikasi; perbarui katalog sebelum generate".into()); }
        (params,cost)
    };
    let lease = acquire(db, &api, ctx, if video { None } else { Some(cost) }).await?;
    let account = &lease.account;
    let mut assets = Vec::new();
    for image in images {
        ctx.progress(0.1, Some(&format!("Mengunggah referensi Vidgen · {}", account.label)));
        let bytes = tokio::fs::read(image).await.map_err(|_| "Gambar referensi tidak bisa dibaca")?;
        let cache_key = format!("vidgen:upload:{}:{}:{:x}",lease.uid,model_id,Sha256::digest(&bytes));
        let cached = { let conn = db.lock().unwrap(); read_cache(&conn, &cache_key).filter(|(_,at)| chrono::Utc::now().timestamp_millis() - at < 3_000_000) };
        if let Some((asset,_)) = cached { assets.push(asset); continue; }
        let mut attempt = 0;
        let asset = loop {
            let result = tokio::select! {_=ctx.token.cancelled()=>return Err("Dibatalkan".into()),r=api.upload(&lease.key,image,model_id)=>r};
            match result {
                Ok(v) => break v,
                Err(e) if attempt < 4 && (e.code == 429 || e.code == 0 || e.code >= 500) => { attempt += 1; pause(&ctx.token, Duration::from_secs(e.retry_after.max(5))).await?; }
                Err(e) => return Err(e.message),
            }
        };
        { let conn = db.lock().unwrap(); write_cache(&conn,&cache_key,&asset)?; }
        assets.push(asset);
    }
    let body = json!({"model":model_id,"prompt":prompt,"parameters":params,"references":assets,"idempotencyKey":ctx.job_id});
    {
        let conn = db.lock().unwrap();
        update_job(
            &conn,
            &ctx.job_id,
            json!({"payload":{"accountId":account.id,"accountLabel":account.label,"idempotencyKey":ctx.job_id}}),
        )?;
    }
    ctx.progress(
        0.14,
        Some(&format!("Mengirim media ke Vidgen · {}", account.label)),
    );
    // Retry stays on this account with the same key; never resubmit an ambiguous job to another account.
    let mut attempt = 0;
    let submitted = loop {
        let result = tokio::select! {_=ctx.token.cancelled()=>return Err("Dibatalkan; periksa riwayat Vidgen sebelum mengulang".into()),r=api.submit(&lease.key,&body,&ctx.token)=>r};
        match result {
            Ok(v) => break v,
            Err(e) if e.code == 429 && attempt < 4 => {
                attempt += 1;
                ctx.progress(0.15, Some("Antrean Vidgen penuh; menunggu slot"));
                pause(&ctx.token, Duration::from_secs(30 * attempt)).await?;
            }
            Err(e) => {
                return Err(format!(
                    "{}; periksa riwayat Vidgen sebelum membuat ulang",
                    e.message
                ))
            }
        }
    };
    let id = submitted["job"]["id"]
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 200
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
        })
        .ok_or("ID job Vidgen tidak valid")?;
    ctx.set_remote(id);
    if submitted["job"]["creditsCharged"].as_f64().unwrap_or(0.0) > cost + 0.0001 {
        return Err("Vidgen melaporkan tagihan kredit pada model Unlimited. Periksa akun dan job sebelum mengulang.".into());
    }
    let deadline = Instant::now() + Duration::from_secs(7200);
    let mut delay = 5;
    let job_path = format!("/api/v1/jobs/{id}");
    loop {
        if Instant::now() > deadline {
            return Err("Job Vidgen melewati dua jam; periksa riwayat sebelum mengulang".into());
        }
        pause(&ctx.token, Duration::from_secs(delay)).await?;
        let value = tokio::select! {_=ctx.token.cancelled()=>return Err("Dibatalkan; job tetap dapat selesai di Vidgen".into()),r=api.get(&job_path,&lease.key)=>r};
        let value = match value {
            Ok(v) => v,
            Err(e) if e.code == 0 || e.code == 429 || e.code >= 500 => {
                delay = 30;
                continue;
            }
            Err(e) => return Err(e.message),
        };
        let job = &value["job"];
        match job["status"].as_str().unwrap_or("") {
            "succeeded" => {
                ctx.progress(0.92, Some("Mengunduh hasil Vidgen"));
                let url = job["result"]["url"]
                    .as_str()
                    .ok_or("Vidgen selesai tanpa URL media")?;
                let bytes = api.download(&lease.key, url, &ctx.token, video).await?;
                return Ok((
                    bytes,
                    json!({"accountId":account.id,"accountLabel":account.label,"remoteId":id}),
                ));
            }
            "failed" | "cancelled" | "canceled" => {
                return Err(job["errorMessage"]
                    .as_str()
                    .unwrap_or("Generasi Vidgen gagal")
                    .replace(&lease.key, "[key]"))
            }
            "queued" => {
                let position = job["queuePosition"]
                    .as_u64()
                    .or(job["queue"]["position"].as_u64());
                ctx.progress(
                    0.18,
                    Some(&format!(
                        "Vidgen · {} · antrean {}",
                        account.label,
                        position
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "menunggu".into())
                    )),
                );
            }
            "dispatching" | "processing" => ctx.progress(
                0.3,
                Some(&format!("Vidgen sedang membuat video · {}", account.label)),
            ),
            _ => {
                return Err(
                    "Status job Vidgen tidak dikenali; periksa riwayat sebelum mengulang".into(),
                )
            }
        }
        delay = (delay * 2).min(30);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn catalog() -> Value {
        json!({"success":true,"models":{"video":[{"id":MODEL,"unlimitedOnly":true,"creditsDefault":0,"capabilities":{"durations":["15"],"ratios":["16:9","9:16"],"resolutions":["720p","1080p"],"maxReferenceImages":3}}]}})
    }
    #[test]
    fn image_pricing_and_paid_accounts_are_distinct_from_unlimited_video() {
        let catalog: Value = serde_json::from_str(include_str!("../../../src/shared/vidgen-catalog.json")).unwrap();
        let model = &catalog["imagePricing"][0];
        let (params,cost) = image_parameters(model,"16:9",1).unwrap();
        assert_eq!(params["resolution"],"2K"); assert!(cost > 0.0);
        let mut info = json!({"user":{"uid":"one","credits":cost},"limits":{"currentActiveJobs":0},"unlimited":{"active":false}});
        assert_eq!(available_image_uid(&info,cost).unwrap(),Some("one"));
        assert!(available_uid(&info).is_err());
        info["user"]["credits"] = json!(0); assert!(available_image_uid(&info,cost).is_err());
        assert!(image_parameters(model,"invalid",0).is_err());
        assert!(image_parameters(model,"16:9",1000).is_err());
    }

    #[test]
    fn model_validation_stops_paid_or_incompatible_requests() {
        assert_eq!(
            crate::services::generation::get_video_model(Some("vidgen:legacy-id")).family,
            "Vidgen"
        );
        assert_eq!(
            parameters(&catalog(), "16:9", Some("720p")).unwrap()["duration"],
            "15"
        );
        assert!(parameters(&catalog(), "4:3", None).is_err());
        assert!(parameters(&catalog(), "16:9", Some("4K")).is_err());
        let mut changed = catalog();
        changed["models"]["video"][0]["creditsDefault"] = json!(2);
        assert!(parameters(&changed, "16:9", None).is_err());
        assert!(parameters(&json!({"models":{"video":[]}}), "16:9", None).is_err());
    }

    #[test]
    fn eligibility_requires_active_unlimited_available_quota_and_account_identity() {
        let ready = json!({"user":{"uid":"one"},"unlimited":{"active":true},"limits":{"dailyVideosRemaining":50,"currentActiveJobs":0}});
        assert_eq!(available_uid(&ready).unwrap(), Some("one"));
        let mut value = ready.clone();
        value["unlimited"]["active"] = json!(false);
        assert!(available_uid(&value).is_err());
        let mut value = ready.clone();
        value["limits"]["dailyVideosRemaining"] = json!(0);
        assert!(available_uid(&value).is_err());
        let mut value = ready.clone();
        value["limits"]["currentActiveJobs"] = json!(1);
        assert_eq!(available_uid(&value).unwrap(), None);
        let mut value = ready.clone();
        value["user"]["uid"] = json!(null);
        assert!(available_uid(&value).is_err());
        assert!(available_uid(&json!({"unlimited":{"active":true}})).is_err());
    }
    #[test]
    fn account_slots_deduplicate_real_users_and_allow_all_connected_accounts() {
        let mut slots = Slots::default();
        assert!(slots.reserve("same-user", "key-one", 0));
        assert!(!slots.reserve("same-user", "key-two", 0));
        for i in 0..25 {
            assert!(slots.reserve(&format!("user-{i}"), &format!("account-{i}"), 0));
        }
        assert_eq!(slots.users.len(), 26);
        assert!(!slots.reserve("new-user", "new-key", 2));
        slots.users.clear();
        assert!(slots.reserve("one", "one", 2));
        assert!(slots.reserve("two", "two", 2));
        assert!(!slots.reserve("three", "three", 2));
        slots.users.remove("one");
        assert!(slots.reserve("three", "three", 2));
    }

    #[test]
    fn account_status_does_not_copy_unknown_private_fields() {
        let info = public_info(
            &json!({"user":{"uid":"one","email":"creator@example.com","apiKey":"must-not-copy"},"unlimited":{"active":true},"secret":"must-not-copy"}),
        );
        assert_eq!(info["user"]["uid"], "one");
        assert!(!info.to_string().contains("must-not-copy"));
    }

    #[tokio::test]
    async fn cancelled_wait_stops_without_a_remote_request() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(pause(&token, Duration::from_secs(60)).await.is_err());
    }
    #[test]
    fn accounts_keep_keys_encrypted_and_do_not_lose_other_accounts() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE cache(key TEXT PRIMARY KEY,value TEXT,updated_at INTEGER);CREATE TABLE secrets(provider TEXT PRIMARY KEY,value BLOB,last_ok INTEGER,last_message TEXT,checked_at INTEGER);").unwrap();
        let db = Arc::new(Mutex::new(conn));
        let first =
            save_account(&db, None, "First".into(), "vg_fake_first_key".into(), true).unwrap();
        let id = first[0]["id"].as_str().unwrap();
        let second = save_account(
            &db,
            None,
            "Second".into(),
            "vg_fake_second_key".into(),
            true,
        )
        .unwrap();
        assert_eq!(second.len(), 2);
        assert!(!serde_json::to_string(&second).unwrap().contains("vg_fake"));
        let name = secret_name(id).unwrap();
        let before: Vec<u8> = db
            .lock()
            .unwrap()
            .query_row("SELECT value FROM secrets WHERE provider=?", [&name], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(!before
            .windows(b"vg_fake_first_key".len())
            .any(|w| w == b"vg_fake_first_key"));
        save_account(
            &db,
            Some(id.into()),
            "First renamed".into(),
            "".into(),
            false,
        )
        .unwrap();
        let after: Vec<u8> = db
            .lock()
            .unwrap()
            .query_row("SELECT value FROM secrets WHERE provider=?", [&name], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(before, after);
        assert_eq!(remove_account(&db, id).unwrap().len(), 1);
        assert!(secret_name("../../other").is_err());
    }

    #[tokio::test]
    async fn image_contract_preserves_pricing_parameters_and_downloads_private_png() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for step in 0..2 {
                let (mut socket,_) = listener.accept().await.unwrap();
                let mut request = Vec::new(); let mut chunk = [0;8192];
                let header_end = loop { let n = socket.read(&mut chunk).await.unwrap(); assert!(n > 0); request.extend_from_slice(&chunk[..n]); if let Some(i) = request.windows(4).position(|b| b == b"\r\n\r\n") { break i+4; } };
                let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
                assert!(headers.contains("authorization: bearer vg_fake_image"));
                let len = headers.lines().find_map(|line| line.strip_prefix("content-length:")).map(|s| s.trim().parse::<usize>().unwrap()).unwrap_or(0);
                while request.len() < header_end + len { let n = socket.read(&mut chunk).await.unwrap(); assert!(n>0); request.extend_from_slice(&chunk[..n]); }
                let (body,kind) = if step == 0 {
                    let value: Value = serde_json::from_slice(&request[header_end..]).unwrap();
                    assert_eq!(value["model"],"gpt-image-2.6-sunburst");
                    assert_eq!(value["parameters"]["ratio"],"16:9");
                    assert_eq!(value["parameters"]["resolution"],"2K");
                    (json!({"success":true,"job":{"id":"image-one","creditsCharged":0.8}}).to_string().into_bytes(),"application/json")
                } else { (b"\x89PNG\r\n\x1a\nimage-transport-fixture".to_vec(),"image/png") };
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {kind}\r\nConnection: close\r\n\r\n",body.len()).as_bytes()).await.unwrap();
                socket.write_all(&body).await.unwrap();
            }
        });
        let mut api = Api::new(); api.base = format!("http://{address}"); let token = CancellationToken::new();
        let catalog: Value = serde_json::from_str(include_str!("../../../src/shared/vidgen-catalog.json")).unwrap();
        let (params,_) = image_parameters(&catalog["imagePricing"][0],"16:9",0).unwrap();
        let body = json!({"model":"gpt-image-2.6-sunburst","prompt":"A harbor","parameters":params,"references":[],"idempotencyKey":"image-test-one"});
        assert_eq!(api.submit("vg_fake_image",&body,&token).await.unwrap()["job"]["id"],"image-one");
        assert!(api.download("vg_fake_image","/private/image.png",&token,false).await.unwrap().starts_with(b"\x89PNG"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn documented_http_contract_upload_idempotent_retry_and_private_download() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut first_submission = None;
            for step in 0..7 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut chunk = [0; 4096];
                let header_end = loop {
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&chunk[..n]);
                    if let Some(i) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&request[..header_end]).to_lowercase();
                let len = headers
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("content-length:")
                            .and_then(|v| v.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                while request.len() < header_end + len {
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&chunk[..n]);
                }
                let path = headers
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap();
                if step != 1 {
                    assert!(headers.contains("authorization: bearer vg_fake_contract"));
                }
                let (status, body, content_type) = match step {
                    0 => {
                        assert_eq!(path, "/api/v1/user/credits");
                        ("200 OK",json!({"success":true,"user":{"uid":"account-a"},"unlimited":{"active":true},"limits":{"dailyVideosRemaining":50}}).to_string().into_bytes(),"application/json")
                    }
                    1 => {
                        assert_eq!(path, "/api/v1/models");
                        (
                            "200 OK",
                            catalog().to_string().into_bytes(),
                            "application/json",
                        )
                    }
                    2 => {
                        assert_eq!(path, "/api/v1/upload");
                        assert!(headers.contains("multipart/form-data"));
                        ("200 OK",json!({"success":true,"asset":{"id":"ref-one","url":"/api/media/ref-one","filename":"ref-one.png","type":"image","opaque":"preserve"}}).to_string().into_bytes(),"application/json")
                    }
                    3 | 4 => {
                        assert_eq!(path, "/api/v1/generate");
                        let value: Value = serde_json::from_slice(&request[header_end..]).unwrap();
                        assert_eq!(value["model"], MODEL);
                        assert_eq!(value["references"][0]["opaque"], "preserve");
                        assert_eq!(value["idempotencyKey"], "stable-job-123");
                        if step == 3 {
                            first_submission = Some(value);
                            (
                                "500 Internal Server Error",
                                json!({"error":"temporary"}).to_string().into_bytes(),
                                "application/json",
                            )
                        } else {
                            assert_eq!(first_submission, Some(value));
                            ("202 Accepted",json!({"success":true,"job":{"id":"job-one","status":"queued","creditsCharged":0}}).to_string().into_bytes(),"application/json")
                        }
                    }
                    5 => {
                        assert_eq!(path, "/api/v1/jobs/job-one");
                        ("200 OK",json!({"success":true,"job":{"id":"job-one","status":"succeeded","result":{"url":"/api/media/result.mp4"}}}).to_string().into_bytes(),"application/json")
                    }
                    _ => {
                        assert_eq!(path, "/api/media/result.mp4");
                        (
                            "200 OK",
                            b"\0\0\0\x18ftypmp42test-video".to_vec(),
                            "video/mp4",
                        )
                    }
                };
                let response=format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n",body.len());
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.write_all(&body).await.unwrap();
            }
        });
        let mut api = Api::new();
        api.base = format!("http://{address}");
        let key = "vg_fake_contract";
        assert_eq!(
            api.get("/api/v1/user/credits", key).await.unwrap()["user"]["uid"],
            "account-a"
        );
        let model = api.get("/api/v1/models", "").await.unwrap();
        let path =
            std::env::temp_dir().join(format!("vidgen-contract-{}.png", uuid::Uuid::new_v4()));
        tokio::fs::write(&path, b"fake image for transport test")
            .await
            .unwrap();
        let asset = api.upload(key, &path, MODEL).await.unwrap();
        let _ = tokio::fs::remove_file(&path).await;
        let token = CancellationToken::new();
        let body = json!({"model":MODEL,"parameters":parameters(&model,"16:9",None).unwrap(),"references":[asset],"idempotencyKey":"stable-job-123"});
        assert_eq!(
            api.submit(key, &body, &token).await.unwrap()["job"]["id"],
            "job-one"
        );
        let job = api.get("/api/v1/jobs/job-one", key).await.unwrap();
        let bytes = api
            .download(key, job["job"]["result"]["url"].as_str().unwrap(), &token, true)
            .await
            .unwrap();
        assert_eq!(&bytes[4..8], b"ftyp");
        assert!(api
            .download(key, "http://example.com/video.mp4", &token, true)
            .await
            .is_err());
        server.await.unwrap();
    }
}
