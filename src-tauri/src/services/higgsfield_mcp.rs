use std::sync::OnceLock;
use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::{Client, Response, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::db::{repo::{clear_secret, get_secret, set_secret}, DbPool};

const SECRET: &str = "higgsfield-mcp";
const SERVER: &str = "https://mcp.higgsfield.ai/mcp";
const ISSUER: &str = "https://clerk.higgsfield.ai";

fn auth_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn client() -> Result<Client, String> {
    Client::builder().user_agent("De-Story/1.1.0")
        .connect_timeout(Duration::from_secs(15)).timeout(Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::none()).build().map_err(|e| e.to_string())
}

#[derive(Serialize, Deserialize)]
struct Credentials {
    client_id: String,
    access_token: String,
    refresh_token: Option<String>,
    expires_at: i64,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    pub configured: bool,
    pub connected: bool,
    pub email: Option<String>,
    pub credits: Option<f64>,
    pub unlimited: bool,
    pub expires_at: Option<String>,
    pub error: Option<String>,
}

async fn oauth_json(res: Response) -> Result<Value, String> {
    let status = res.status();
    let body: Value = res.json().await.map_err(|_| format!("Login Higgsfield merespons HTTP {status} tanpa JSON"))?;
    if !status.is_success() {
        // OAuth responses may contain credentials; return only the protocol error code.
        let code = body["error"].as_str().unwrap_or("oauth_error");
        return Err(format!("Login Higgsfield gagal ({status}, {code}). Coba hubungkan akun lagi."));
    }
    Ok(body)
}

fn credentials(body: &Value, client_id: String, old_refresh: Option<String>) -> Result<Credentials, String> {
    Ok(Credentials {
        client_id,
        access_token: body["access_token"].as_str().filter(|s| !s.is_empty())
            .ok_or("Login Higgsfield tidak mengembalikan access token")?.to_string(),
        refresh_token: body["refresh_token"].as_str().map(str::to_string).or(old_refresh),
        expires_at: chrono::Utc::now().timestamp() + body["expires_in"].as_i64().unwrap_or(3600),
    })
}

fn callback_code(target: &str, state: &str) -> Result<String, String> {
    let url = Url::parse(&format!("http://127.0.0.1{target}")).map_err(|_| "Callback login tidak valid")?;
    let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    if url.path() != "/callback" || q.get("state").map(String::as_str) != Some(state) {
        return Err("Callback login tidak cocok. Coba hubungkan akun lagi.".into());
    }
    if q.get("iss").is_some_and(|s| s != ISSUER) {
        return Err("Penerbit login Higgsfield tidak cocok".into());
    }
    if q.contains_key("error") {
        return Err("Login Higgsfield dibatalkan atau ditolak".into());
    }
    q.get("code").filter(|s| !s.is_empty()).cloned().ok_or("Kode login Higgsfield tidak ditemukan".into())
}

pub async fn connect(db: &DbPool) -> Result<McpStatus, String> {
    let _guard = auth_lock().try_lock().map_err(|_| "Login Higgsfield masih berlangsung")?;
    let client = client()?;
    let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| e.to_string())?;
    let redirect = format!("http://127.0.0.1:{}/callback", listener.local_addr().map_err(|e| e.to_string())?.port());
    let registration = oauth_json(client.post(format!("{ISSUER}/oauth/register")).json(&json!({
        "client_name": "De Story", "redirect_uris": [redirect],
        "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"],
        "token_endpoint_auth_method": "none", "scope": "openid email offline_access"
    })).send().await.map_err(|e| e.without_url().to_string())?).await?;
    let client_id = registration["client_id"].as_str().ok_or("Pendaftaran koneksi MCP gagal")?.to_string();
    let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = Uuid::new_v4().to_string();
    let mut url = Url::parse(&format!("{ISSUER}/oauth/authorize")).unwrap();
    url.query_pairs_mut().extend_pairs([
        ("client_id", client_id.as_str()), ("redirect_uri", &redirect), ("response_type", "code"),
        ("scope", "openid email offline_access"), ("state", &state),
        ("code_challenge", &challenge), ("code_challenge_method", "S256"), ("resource", SERVER),
    ]);
    open::that(url.as_str()).map_err(|e| format!("Gagal membuka halaman login: {e}"))?;
    let code = tokio::time::timeout(Duration::from_secs(300), async {
        loop {
            let (mut socket, _) = listener.accept().await.map_err(|e| e.to_string())?;
            let mut bytes = Vec::new();
            let read = tokio::time::timeout(Duration::from_secs(5), async {
                while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                    let mut chunk = [0; 1024];
                    let n = socket.read(&mut chunk).await?;
                    if n == 0 || bytes.len() + n > 8192 { return Ok::<bool, std::io::Error>(false); }
                    bytes.extend_from_slice(&chunk[..n]);
                }
                Ok(true)
            }).await;
            if !matches!(read, Ok(Ok(true))) { continue; }
            let request = String::from_utf8_lossy(&bytes);
            let target = request.lines().next().and_then(|l| l.strip_prefix("GET ")).and_then(|l| l.split(' ').next()).unwrap_or("");
            if !target.starts_with("/callback?") { continue; }
            let result = callback_code(target, &state);
            let message = if result.is_ok() { "Login diterima. Silakan kembali ke De Story untuk memeriksa sambungan." } else { "Login tidak cocok atau ditolak. Silakan kembali ke De Story." };
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{message}", message.len());
            let _ = socket.write_all(response.as_bytes()).await;
            return result;
        }
    }).await.map_err(|_| "Waktu login habis. Klik Hubungkan akun untuk mencoba lagi.")??;
    let body = oauth_json(client.post(format!("{ISSUER}/oauth/token")).form(&[
        ("grant_type", "authorization_code"), ("code", &code), ("client_id", &client_id),
        ("redirect_uri", &redirect), ("code_verifier", &verifier), ("resource", SERVER),
    ]).send().await.map_err(|e| e.without_url().to_string())?).await?;
    let creds = credentials(&body, client_id, None)?;
    {
        let conn = db.lock().unwrap();
        set_secret(&conn, SECRET, &serde_json::to_string(&creds).map_err(|e| e.to_string())?)?;
    }
    drop(_guard);
    Ok(status(db).await)
}

async fn access_token(db: &DbPool) -> Result<String, String> {
    let _guard = auth_lock().lock().await;
    let raw = { let conn = db.lock().unwrap(); get_secret(&conn, SECRET) }
        .ok_or("Hubungkan akun Higgsfield MCP di Pengaturan terlebih dahulu")?;
    let mut creds: Credentials = serde_json::from_str(&raw).map_err(|_| "Sesi Higgsfield tidak terbaca. Hubungkan akun lagi.")?;
    if creds.expires_at < chrono::Utc::now().timestamp() + 120 {
        let refresh = creds.refresh_token.clone().ok_or("Sesi Higgsfield habis. Hubungkan akun lagi.")?;
        let body = oauth_json(client()?.post(format!("{ISSUER}/oauth/token")).form(&[
            ("grant_type", "refresh_token"), ("refresh_token", &refresh), ("client_id", &creds.client_id), ("resource", SERVER),
        ]).send().await.map_err(|e| e.without_url().to_string())?).await?;
        creds = credentials(&body, creds.client_id, Some(refresh))?;
        let conn = db.lock().unwrap();
        set_secret(&conn, SECRET, &serde_json::to_string(&creds).map_err(|e| e.to_string())?)?;
    }
    Ok(creds.access_token)
}

pub async fn disconnect(db: &DbPool) -> Result<(), String> {
    let _guard = auth_lock().lock().await;
    let conn = db.lock().unwrap();
    clear_secret(&conn, SECRET)
}

fn rpc_result(root: Value, id: &str) -> Result<Option<Value>, String> {
    if root["id"].as_str() != Some(id) { return Ok(None); }
    if let Some(error) = root.get("error") {
        return Err(error["message"].as_str().unwrap_or("Permintaan MCP ditolak").to_string());
    }
    root.get("result").cloned().map(Some).ok_or("Respons MCP tidak memiliki result".into())
}

fn sse_result(text: &str, id: &str) -> Result<Option<Value>, String> {
    for event in text.replace("\r\n", "\n").split("\n\n") {
        let data = event.lines().filter_map(|l| l.strip_prefix("data:").map(str::trim_start)).collect::<Vec<_>>().join("\n");
        if let Ok(root) = serde_json::from_str::<Value>(&data) {
            if let Some(result) = rpc_result(root, id)? { return Ok(Some(result)); }
        }
    }
    Ok(None)
}

fn tool_result(result: Value) -> Result<Value, String> {
    let value = result.get("structuredContent").cloned().or_else(|| {
        result["content"].as_array()?.iter().filter_map(|b| b["text"].as_str())
            .find_map(|t| serde_json::from_str::<Value>(t).ok())
    });
    if result["isError"].as_bool() == Some(true) {
        let message = value.as_ref().and_then(|v| v["error"].as_str()).or_else(|| result["content"][0]["text"].as_str()).unwrap_or("MCP menolak permintaan");
        return Err(message.to_string());
    }
    let value = value.ok_or("Higgsfield MCP tidak mengembalikan hasil JSON")?;
    if let Some(error) = value["error"].as_str() { return Err(error.to_string()); }
    Ok(value)
}

pub struct Session {
    db: DbPool,
    client: Client,
    token: String,
    session: Option<String>,
    protocol: String,
}

impl Session {
    pub async fn new(db: &DbPool) -> Result<Self, String> {
        let mut s = Self { db: db.clone(), client: client()?, token: access_token(db).await?, session: None, protocol: "2025-03-26".into() };
        let result = s.rpc("initialize", json!({
            "protocolVersion": s.protocol, "capabilities": {}, "clientInfo": { "name": "de-story", "version": "1.1.0" }
        })).await?;
        if let Some(version) = result["protocolVersion"].as_str() { s.protocol = version.to_string(); }
        let mut req = s.client.post(SERVER).bearer_auth(&s.token)
            .header("Accept", "application/json, text/event-stream").header("MCP-Protocol-Version", &s.protocol)
            .json(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        if let Some(session) = &s.session { req = req.header("Mcp-Session-Id", session); }
        let res = req.send().await.map_err(|e| e.without_url().to_string())?;
        if !res.status().is_success() { return Err(format!("Inisialisasi MCP gagal: HTTP {}", res.status())); }
        Ok(s)
    }

    async fn rpc(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = Uuid::new_v4().to_string();
        let mut req = self.client.post(SERVER).bearer_auth(&self.token)
            .header("Accept", "application/json, text/event-stream")
            .json(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        if method != "initialize" { req = req.header("MCP-Protocol-Version", &self.protocol); }
        if let Some(session) = &self.session { req = req.header("Mcp-Session-Id", session); }
        let mut res = req.send().await.map_err(|e| e.without_url().to_string())?;
        if res.status().as_u16() == 401 { return Err("Sesi MCP ditolak. Hubungkan akun Higgsfield lagi di Pengaturan.".into()); }
        if !res.status().is_success() { return Err(format!("Higgsfield MCP gagal: HTTP {}", res.status())); }
        if let Some(session) = res.headers().get("Mcp-Session-Id").and_then(|s| s.to_str().ok()) { self.session = Some(session.to_string()); }
        let sse = res.headers().get("Content-Type").and_then(|s| s.to_str().ok()).unwrap_or("").contains("text/event-stream");
        if !sse {
            return rpc_result(res.json().await.map_err(|_| "Respons JSON MCP tidak valid")?, &id)?.ok_or("ID respons MCP tidak cocok".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = res.chunk().await.map_err(|e| e.without_url().to_string())? {
            bytes.extend_from_slice(&chunk);
            if bytes.len() > 4 * 1024 * 1024 { return Err("Respons MCP terlalu besar".into()); }
            if let Some(result) = sse_result(&String::from_utf8_lossy(&bytes), &id)? { return Ok(result); }
        }
        Err("Sambungan MCP terputus sebelum hasil diterima".into())
    }

    pub async fn tool(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        self.token = access_token(&self.db).await?;
        tool_result(self.rpc("tools/call", json!({ "name": name, "arguments": arguments })).await?)
    }

    pub async fn upload(&mut self, path: &std::path::Path) -> Result<String, String> {
        let filename = path.file_name().and_then(|s| s.to_str()).ok_or("Nama file referensi tidak valid")?;
        let mime = mime_guess::from_path(path).first_or_octet_stream().to_string();
        let slot = self.tool("media_upload", json!({ "filename": filename, "content_type": mime })).await?;
        let upload = &slot["uploads"][0];
        let url = upload["upload_url"].as_str().ok_or("MCP tidak mengembalikan URL upload")?;
        if !url.starts_with("https://") { return Err("URL upload MCP harus HTTPS".into()); }
        let id = upload["media_id"].as_str().ok_or("MCP tidak mengembalikan ID upload")?.to_string();
        let bytes = tokio::fs::read(path).await.map_err(|e| format!("Gagal membaca referensi: {e}"))?;
        let res = self.client.put(url).header("Content-Type", &mime).body(bytes).send().await.map_err(|e| e.without_url().to_string())?;
        if !res.status().is_success() { return Err(format!("Upload referensi gagal: HTTP {}", res.status())); }
        let result = self.tool("media_confirm", json!({ "media_id": id, "type": "image" })).await?;
        if result["results"].as_array().is_none_or(|a| !a.iter().any(|r| r["media_id"].as_str() == Some(&id))) {
            return Err("MCP belum mengonfirmasi file referensi".into());
        }
        Ok(id)
    }

    pub async fn submit(&mut self, kind: &str, model: &str, body: &Value, refs: &[String], role: &str) -> Result<String, String> {
        let params = generation_params(model, body, refs, role)?;
        let result = self.tool(if kind == "video" { "generate_video" } else { "generate_image" }, json!({ "params": params })).await?;
        if result.get("unlim_choice").is_some() {
            return Err("Higgsfield meminta pilihan pembayaran meski Unlimited sudah dipilih. Tidak ada generasi yang dikirim.".into());
        }
        result["results"][0]["id"].as_str().map(str::to_string)
            .ok_or("Higgsfield MCP tidak mengembalikan ID generasi. Periksa Assets sebelum mencoba ulang.".into())
    }
}

fn generation_params(model: &str, body: &Value, refs: &[String], role: &str) -> Result<Value, String> {
    let model = model.strip_prefix("hfmcp:").ok_or("Model Higgsfield MCP tidak valid")?;
    let mut params = body.clone();
    let p = params.as_object_mut().ok_or("Parameter generasi harus berupa objek")?;
    p.remove("image_references"); p.remove("start_image");
    p.remove("batch_size"); p.remove("num_images");
    p.insert("model".into(), json!(model));
    p.insert("count".into(), json!(1));
    // The user chose the MCP trial. Never retry on paid credits when it is unavailable.
    p.insert("use_unlim".into(), json!(true));
    if !refs.is_empty() {
        p.insert("medias".into(), json!(refs.iter().map(|id| json!({ "role": role, "value": id })).collect::<Vec<_>>()));
    }
    Ok(params)
}

pub async fn status(db: &DbPool) -> McpStatus {
    let configured = { let conn = db.lock().unwrap(); get_secret(&conn, SECRET).is_some() };
    if !configured { return McpStatus::default(); }
    let mut status = McpStatus { configured: true, ..Default::default() };
    let result = async {
        let mut session = Session::new(db).await?;
        let profile = session.tool("get_profile", json!({})).await?;
        let balance = session.tool("balance", json!({})).await?;
        status.connected = true;
        status.email = profile["email"].as_str().map(str::to_string);
        status.credits = balance["credits"].as_f64();
        status.unlimited = balance["free_trial"]["unlim_available"].as_bool().unwrap_or(false);
        status.expires_at = balance["free_trial"]["unlim_expires_at"].as_str().map(str::to_string);
        Ok::<(), String>(())
    }.await;
    if let Err(e) = result { status.error = Some(e); }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_validates_state_and_issuer() {
        assert_eq!(callback_code("/callback?code=ok&state=abc&iss=https%3A%2F%2Fclerk.higgsfield.ai", "abc").unwrap(), "ok");
        assert!(callback_code("/callback?code=ok&state=wrong", "abc").is_err());
        assert!(callback_code("/callback?code=ok&state=abc&iss=https%3A%2F%2Fother.test", "abc").is_err());
        assert!(callback_code("/callback?error=access_denied&state=abc", "abc").is_err());
        assert!(callback_code("/other?code=ok&state=abc", "abc").is_err());
    }

    #[test]
    fn sse_ignores_notifications_and_partial_chunks() {
        let first = "event: message\r\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\r\n\r\n";
        assert!(sse_result(first, "abc").unwrap().is_none());
        assert!(sse_result(&format!("{first}data: {{\"id\":\"abc\",\"res"), "abc").unwrap().is_none());
        assert_eq!(sse_result(&format!("{first}data: {{\"id\":\"abc\",\"result\":{{\"ok\":true}}}}\n\n"), "abc").unwrap().unwrap()["ok"], true);
    }

    #[test]
    fn tools_accept_structured_or_text_json_and_reject_errors() {
        assert_eq!(tool_result(json!({"structuredContent":{"credits":12}})).unwrap()["credits"], 12);
        assert_eq!(tool_result(json!({"content":[{"type":"text","text":"{\"credits\":12}"}]})).unwrap()["credits"], 12);
        assert!(tool_result(json!({"isError":true,"content":[{"text":"trial expired"}]})).is_err());
        assert!(tool_result(json!({"structuredContent":{"error":"no allowance"}})).is_err());
    }

    #[test]
    fn generation_always_uses_unlimited_and_maps_media() {
        let p = generation_params("hfmcp:seedance_2_5", &json!({"prompt":"hello","mode":"omni_reference","batch_size":4,"start_image":"local"}), &["uuid".into()], "start_image").unwrap();
        assert_eq!(p["use_unlim"], true);
        assert_eq!(p["count"], 1);
        assert_eq!(p["model"], "seedance_2_5");
        assert_eq!(p["mode"], "omni_reference");
        assert_eq!(p["medias"][0]["role"], "start_image");
        assert!(p.get("start_image").is_none());
        assert!(p.get("batch_size").is_none());
        assert!(generation_params("flow:model", &json!({}), &[], "image").is_err());
    }

    #[test]
    fn refresh_preserves_rotating_token_when_omitted() {
        let c = credentials(&json!({"access_token":"a","expires_in":3600}), "client".into(), Some("old".into())).unwrap();
        assert_eq!(c.refresh_token.as_deref(), Some("old"));
        assert!(credentials(&json!({}), "client".into(), None).is_err());
    }
}
