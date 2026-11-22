use crate::flow2api_runtime::Flow2ApiPanelInfo;
use serde_json::{json,Value};

pub async fn request(info: &Flow2ApiPanelInfo, action: Option<Value>) -> Result<Value,String> {
    if let Some(ref body) = action {
        let name = body["action"].as_str().ok_or("Pilih tindakan sesi Flow")?;
        if !["add","open","complete","close","renew","renew-all","attach-all","remove","config"].contains(&name) { return Err("Tindakan sesi Flow tidak dikenal".into()); }
        if let Some(id) = body["profileId"].as_str() { uuid::Uuid::parse_str(id).map_err(|_|"ID profil Flow tidak valid")?; }
    }
    let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(std::time::Duration::from_secs(120)).build().map_err(|e|e.to_string())?;
    let login: Value = client.post(format!("{}/api/admin/login",info.base_url)).json(&json!({"username":info.username,"password":info.password})).send().await
        .map_err(|_|"Panel Flow2API tidak dapat dihubungi")?.error_for_status().map_err(|_|"Login panel Flow2API gagal")?.json().await.map_err(|_|"Respons panel tidak valid")?;
    let auth = login["token"].as_str().ok_or("Token panel tidak tersedia")?;
    let url = format!("{}/api/studio/sessions",info.base_url);
    let query = if let Some(body) = action { client.post(url).json(&body) } else {client.get(url)}.bearer_auth(auth);
    let result = async {
        let response = query.send().await.map_err(|_|"Pengaturan sesi Flow belum merespons. Pastikan Flow2API siap; bila baru diperbarui, buka ulang Bara Studio.")?;
        let status = response.status();
        let value: Value = response.json().await.map_err(|_|"Versi backend belum menyediakan pengaturan sesi. Buka ulang aplikasi setelah pembaruan.")?;
        if !status.is_success() { return Err(value["detail"].as_str().unwrap_or("Pengaturan sesi belum berhasil. Periksa kesiapan backend.").chars().take(300).collect()); }
        Ok(value)
    }.await;
    let _ = client.post(format!("{}/api/admin/logout",info.base_url)).bearer_auth(auth).send().await;
    result
}
