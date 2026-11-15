use std::collections::HashMap;
use std::fs;
use std::sync::{Arc, OnceLock};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use crate::db::repo::{
    active_job_for, characters_of, clips_of, drop_cache, get_asset, get_character, get_clip,
    get_project, get_secret, get_settings, insert_asset, insert_job, patch_clip,
    read_cache, set_character_sheet, touch_project, update_asset_meta, update_job, write_cache,
    AssetRow, CharacterRow, NewAsset,
};
use crate::db::DbPool;
use crate::jobs::{JobManager, QueueName, TaskCtx};
use crate::paths::asset_abs_path;
use crate::services::audio::{probe_duration_ms, probe_size};
use crate::services::http::download_to;
use crate::services::{elevenlabs, gemini, higgsfield as hf, higgsfield_mcp as mcp, styles};

use crate::speech::align::{estimate_word_timings, strip_voice_tags};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// The supervised Flow2API backend, when the app has one. Generation tasks run
/// outside a command, so they reach the manager through the app handle.
fn flow2api_manager(app: &AppHandle) -> Option<Arc<crate::flow2api_runtime::Flow2ApiManager>> {
    use tauri::Manager;
    app.try_state::<Arc<crate::flow2api_runtime::Flow2ApiManager>>().map(|s| s.inner().clone())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HfInput {
    pub field: Option<String>,
    pub mode: String, // "none" | "first-frame" | "reference"
    pub max: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HfModel {
    pub id: String,
    pub name: String,
    pub family: String,
    pub kind: String, // "image" | "video"
    pub input: HfInput,
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub props: HashMap<String, Value>,
    pub durations: Option<Value>,
    #[serde(default)]
    pub tags: Vec<String>,
}

pub fn get_catalog_models() -> Vec<HfModel> {
    [include_str!("../../../src/shared/higgsfield-catalog.json"), include_str!("../../../src/shared/higgsfield-mcp-catalog.json"), include_str!("../../../src/shared/vidgen-catalog.json")]
        .iter().flat_map(|s| {
            let v: Value = serde_json::from_str(s).unwrap_or_default();
            serde_json::from_value::<Vec<HfModel>>(v["models"].clone()).unwrap_or_default()
        }).collect()
}

pub fn get_image_model(id: Option<&str>) -> HfModel {
    if let Some(target) = id {
        if target.starts_with("flow:") {
            let canonical = crate::services::flow2api::canonical_flow_id(target);
            if let Some(m) = crate::services::flow2api::flow_image_models().into_iter().find(|m| m.id == canonical) {
                return m;
            }
        }
    }
    let models = get_catalog_models();
    if let Some(target) = id {
        if let Some(m) = models.iter().find(|m| m.kind == "image" && m.id == target) {
            return m.clone();
        }
    }
    models
        .into_iter()
        .find(|m| m.id == if id.is_some_and(|s| s.starts_with("vidgen:")) { "vidgen:gpt-image-2.6-sunburst" } else if id.is_some_and(|s| s.starts_with("hfmcp:")) { "hfmcp:gpt_image_2_6" } else { "marketing-studio/image/sunburst" })
        .expect("Default image model not found in catalog")
}

pub fn get_video_model(id: Option<&str>) -> HfModel {
    if let Some(target) = id {
        if target.starts_with("flow:") {
            let canonical = crate::services::flow2api::canonical_flow_id(target);
            if let Some(m) = crate::services::flow2api::flow_video_models().into_iter().find(|m| m.id == canonical) {
                return m;
            }
        }
    }
    let models = get_catalog_models();
    if let Some(target) = id {
        if let Some(m) = models.iter().find(|m| m.kind == "video" && m.id == target) {
            return m.clone();
        }
    }
    models
        .into_iter()
        .find(|m| m.id == if id.is_some_and(|s| s.starts_with("vidgen:")) { "vidgen:gemini-omni-1.2-unlimited" } else if id.is_some_and(|s| s.starts_with("hfmcp:")) { "hfmcp:seedance_2_5" } else { "kling-video/v3.0/std/image-to-video" })
        .expect("Default video model not found in catalog")
}

pub fn max_refs(m: &HfModel) -> usize {
    if m.kind == "image" && m.input.mode == "reference" {
        m.input.max
    } else {
        0
    }
}

pub fn prompt_limit(m: &HfModel) -> usize {
    m.props
        .get("prompt")
        .and_then(|p| p.get("maxLength"))
        .and_then(|l| l.as_u64())
        .map(|l| l as usize)
        .unwrap_or(if m.kind == "video" { 2500 } else { 4000 })
}

pub fn fit_duration(m: &HfModel, seconds: f64) -> Option<f64> {
    let d = m.durations.as_ref()?;
    let want = seconds.ceil().max(1.0);
    if let Some(arr) = d.get("values").and_then(|v| v.as_array()) {
        let mut vals: Vec<f64> = arr.iter().filter_map(|x| x.as_f64()).collect();
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for v in &vals {
            if *v >= want {
                return Some(*v);
            }
        }
        return vals.last().copied();
    }
    if let (Some(min_v), Some(max_v)) = (
        d.get("min").and_then(|x| x.as_f64()),
        d.get("max").and_then(|x| x.as_f64()),
    ) {
        return Some(want.clamp(min_v, max_v));
    }
    None
}

fn clip_str(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    // Snap down to a char boundary: slicing mid-codepoint panics on multibyte text.
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let cut = &text[..end];
    if let Some(space) = cut.rfind(' ') {
        if space > (limit * 8 / 10) {
            return cut[..space].trim_end();
        }
    }
    cut.trim_end()
}

fn resolution_for(m: &HfModel, want: Option<&str>) -> Option<Value> {
    let res_prop = m.props.get("resolution")?;
    let enums = res_prop.get("enum").and_then(|e| e.as_array())?;

    if let Some(w) = want {
        if let Some(hit) = enums.iter().find(|v| v.as_str() == Some(w)) {
            return Some(hit.clone());
        }
    }

    let prefer = if m.kind == "image" {
        vec!["2k", "2K", "1080p", "1k"]
    } else {
        vec!["1080p", "2K", "2k", "720p"]
    };

    for p in prefer {
        if let Some(hit) = enums.iter().find(|v| v.as_str().map(|s| s.to_lowercase()) == Some(p.to_lowercase())) {
            return Some(hit.clone());
        }
    }

    enums.first().cloned()
}

fn common_body(m: &HfModel, prompt: &str, aspect: &str, resolution: Option<&str>) -> Value {
    let mut body = json!({
        "prompt": clip_str(prompt, prompt_limit(m))
    });

    let obj = body.as_object_mut().unwrap();

    if let Some(asp_prop) = m.props.get("aspect_ratio") {
        if let Some(enums) = asp_prop.get("enum").and_then(|e| e.as_array()) {
            if enums.iter().any(|v| v.as_str() == Some(aspect)) {
                obj.insert("aspect_ratio".to_string(), json!(aspect));
            }
        } else {
            obj.insert("aspect_ratio".to_string(), json!(aspect));
        }
    }

    if let Some(res) = resolution_for(m, resolution) {
        obj.insert("resolution".to_string(), res);
    }

    if let Some(q_prop) = m.props.get("quality") {
        if let Some(enums) = q_prop.get("enum").and_then(|e| e.as_array()) {
            let val = if enums.iter().any(|v| v.as_str() == Some("high")) {
                "high"
            } else {
                enums.last().and_then(|v| v.as_str()).unwrap_or("high")
            };
            obj.insert("quality".to_string(), json!(val));
        }
    }

    if m.props.contains_key("batch_size") {
        obj.insert("batch_size".to_string(), json!(1));
    }
    if m.props.contains_key("num_images") {
        obj.insert("num_images".to_string(), json!(1));
    }
    if m.props.contains_key("enhance_prompt") {
        obj.insert("enhance_prompt".to_string(), json!(false));
    }
    if m.props.contains_key("prompt_extend") {
        obj.insert("prompt_extend".to_string(), json!(false));
    }
    if m.props.contains_key("generate_audio") {
        obj.insert("generate_audio".to_string(), json!(false));
    }

    if m.id.starts_with("hfmcp:") {
        if let Some(options) = m.props.get("mode").and_then(|p| p["enum"].as_array()) {
            let mode = ["omni_reference", "image-to-video"].iter()
                .find_map(|want| options.iter().find(|v| v.as_str() == Some(want)))
                .or_else(|| m.props["mode"].get("default"));
            if let Some(mode) = mode { obj.insert("mode".into(), mode.clone()); }
        }
        if m.props.get("sound").and_then(|p| p["enum"].as_array()).is_some_and(|v| v.contains(&json!("off"))) {
            obj.insert("sound".into(), json!("off"));
        }
    }

    body
}

pub fn image_body(m: &HfModel, prompt: &str, aspect: &str, refs: &[String]) -> Result<Value, String> {
    let mut body = common_body(m, prompt, aspect, None);
    let limit = max_refs(m);
    if limit > 0 && !refs.is_empty() {
        if let Some(ref field) = m.input.field {
            let slice: Vec<String> = refs.iter().take(limit).cloned().collect();
            body.as_object_mut().unwrap().insert(field.clone(), json!(slice));
        }
    }

    // Verify required fields
    for req in &m.required {
        if body.get(req).is_none() {
            return Err(format!("Model {} butuh field '{}' yang belum didukung.", m.name, req));
        }
    }

    Ok(body)
}

pub fn video_body(
    m: &HfModel,
    prompt: &str,
    image_url: &str,
    seconds: f64,
    aspect: &str,
    resolution: Option<&str>,
) -> Result<Value, String> {
    let full_prompt = format!("{prompt} Silent clip: no background music, no soundtrack, no singing or humming.");
    let mut body = common_body(m, &full_prompt, aspect, resolution);

    if let Some(dur) = fit_duration(m, seconds) {
        body.as_object_mut().unwrap().insert("duration".to_string(), json!(dur));
    }

    if let Some(ref field) = m.input.field {
        let val = if m.input.mode == "reference" {
            json!([image_url])
        } else {
            json!(image_url)
        };
        body.as_object_mut().unwrap().insert(field.clone(), val);
    }

    for req in &m.required {
        if body.get(req).is_none() {
            return Err(format!("Model {} butuh field '{}' yang belum didukung.", m.name, req));
        }
    }

    Ok(body)
}

fn split_avoid(visual: &str) -> (String, String) {
    let mut lines: Vec<&str> = visual.lines().collect();
    let mut avoid = String::new();
    let mut avoid_idx = None;

    for (i, l) in lines.iter().enumerate() {
        let trimmed = l.trim_start();
        if trimmed.to_lowercase().starts_with("avoid:") {
            avoid = trimmed[6..].trim().trim_end_matches('.').to_string();
            avoid_idx = Some(i);
            break;
        }
    }

    if let Some(idx) = avoid_idx {
        lines.remove(idx);
    }

    (lines.join("\n").trim().to_string(), avoid)
}

fn clip_prompt(
    aspect: &str,
    style: &str,
    visual: &str,
    chars: &[CharacterRow],
    ref_names: &[String],
    limit: usize,
) -> String {
    let (scene, avoid) = split_avoid(visual);
    let wardrobe = scene.to_lowercase().contains("wardrobe:");

    // Style first so a truncated prompt never loses it.
    let mut parts = vec![
        format!("Art style (must match exactly): {style}"),
        format!("Scene:\n{scene}"),
    ];

    if !chars.is_empty() {
        let mut char_text = format!(
            "Recurring characters in this scene. Keep each one's body, face, hair and skin exactly as described{}.",
            if !ref_names.is_empty() { " and as on their reference sheet" } else { "" }
        );
        if wardrobe {
            char_text.push_str(" Their clothing follows the Wardrobe line of the scene: the default outfit only where it says so, otherwise the custom outfit.");
        } else {
            char_text.push_str(" They wear their default outfit.");
        }
        char_text.push_str("\n");
        char_text.push_str(&chars.iter().map(|c| format!("{}: {}", c.name, c.description)).collect::<Vec<_>>().join("\n"));
        parts.push(char_text);
    }

    if !ref_names.is_empty() {
        let mut ref_text = format!("Reference images are character sheets, in this order: {}. Use them for each character's identity, proportions and colors, but render them in the requested art style even if a reference sheet uses a different style. ", ref_names.join(", "));
        if wardrobe {
            ref_text.push_str("Copy the costume from a sheet only for characters whose Wardrobe line says \"default outfit\". ");
        } else {
            ref_text.push_str("Match the costume to the sheet. ");
        }
        ref_text.push_str("Do not copy the sheet layout, poses or plain background.");
        parts.push(ref_text);
    }

    if !avoid.is_empty() {
        parts.push(format!("Must not appear in the image: {avoid}."));
    }
    parts.push(format!("Frame format {aspect}, full-bleed composition."));

    let full = parts.join("\n\n");
    if full.len() <= limit {
        full
    } else {
        clip_str(&full, limit).to_string()
    }
}

fn sheet_prompt(c: &CharacterRow, style: &str, hint: &str) -> String {
    format!(
        "Character reference sheet for \"{}\": {}. \
Draw the character as {}. \
Show the SAME character three times side by side: front view, three-quarter view and side view, \
full body, neutral standing pose, evenly spaced, on a simple uncluttered background that does not override the selected art style. \
Art style of every view: {style}. This style determines the character's visual form and rendering; keep the character's identity and costume details. \
Consistent proportions, costume and colors in all three views.",
        c.name, c.description, hint
    )
}

async fn public_url_for(
    key: &str,
    db: &DbPool,
    asset: &AssetRow,
) -> Result<String, String> {
    let now = chrono::Utc::now().timestamp_millis();
    if let Some(ref r_url) = asset.remote_url {
        if asset.provider.as_deref() == Some("higgsfield") && (now - asset.created_at) < 6 * DAY_MS {
            return Ok(r_url.clone());
        }
    }

    if let (Some(up), Some(at)) = (
        asset.meta.get("uploadedUrl").and_then(|x| x.as_str()),
        asset.meta.get("uploadedAt").and_then(|x| x.as_i64()),
    ) {
        if now - at < (DAY_MS / 2) {
            return Ok(up.to_string());
        }
    }

    let abs = asset_abs_path(&asset.project_id, &asset.local_path);
    let url = hf::upload_file(key, &abs).await?;

    let mut meta = asset.meta.clone();
    if let Some(obj) = meta.as_object_mut() {
        obj.insert("uploadedUrl".to_string(), json!(url));
        obj.insert("uploadedAt".to_string(), json!(now));
    }

    let conn = db.lock().unwrap();
    let _ = update_asset_meta(&conn, &asset.id, meta);

    Ok(url)
}

async fn wait_for_character_sheets(
    db: &DbPool,
    char_ids: &[String],
    token: &tokio_util::sync::CancellationToken,
) {
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(15 * 60) {
        if token.is_cancelled() {
            return;
        }
        let pending = {
            let conn = db.lock().unwrap();
            char_ids.iter().any(|cid| active_job_for(&conn, None, Some(cid), "sheet").is_some())
        };
        if !pending {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
    }
}

async fn reference_urls(
    key: &str,
    db: &DbPool,
    model: &HfModel,
    chars: &[CharacterRow],
    token: &tokio_util::sync::CancellationToken,
) -> (Vec<String>, Vec<String>) {
    let limit = max_refs(model);
    if limit == 0 {
        return (Vec::new(), Vec::new());
    }

    let c_ids: Vec<String> = chars.iter().map(|c| c.id.clone()).collect();
    wait_for_character_sheets(db, &c_ids, token).await;

    let mut urls = Vec::new();
    let mut names = Vec::new();

    for c in chars {
        if urls.len() >= limit {
            break;
        }
        let sheet_opt = {
            let conn = db.lock().unwrap();
            get_character(&conn, &c.id).and_then(|fresh| fresh.sheet_asset_id).and_then(|sid| get_asset(&conn, &sid))
        };

        if let Some(sheet) = sheet_opt {
            if let Ok(u) = public_url_for(key, db, &sheet).await {
                urls.push(u);
                names.push(c.name.clone());
            }
        }
    }

    (urls, names)
}

// Shared by Flow images, sheets and videos, including manual generation.
// Acquire after waiting for references and hold until the asset is saved.
#[derive(Default)]
struct FlowMediaGate {
    active: std::sync::Mutex<usize>,
    changed: tokio::sync::Notify,
}

struct FlowMediaTurn<'a>(&'a FlowMediaGate);

impl Drop for FlowMediaTurn<'_> {
    fn drop(&mut self) {
        *self.0.active.lock().unwrap() -= 1;
        self.0.changed.notify_waiters();
    }
}

fn flow_media_gate() -> &'static FlowMediaGate {
    static GATE: OnceLock<FlowMediaGate> = OnceLock::new();
    GATE.get_or_init(FlowMediaGate::default)
}

async fn flow_parallel_accounts(db: &DbPool, app: &AppHandle, video: bool) -> Result<usize, String> {
    let settings = { let conn = db.lock().unwrap(); get_settings(&conn) };
    if settings["flow2apiMode"].as_str() == Some("external") { return Ok(1); }
    let selected = settings["flow2apiParallelAccounts"].as_u64().unwrap_or(1) as u32;
    let manager = flow2api_manager(app).ok_or("Flow2API internal belum siap")?;
    let pool = crate::services::flow2api::account_pool(&manager.panel_info()?, Some(selected)).await?;
    Ok(pool.limit(selected, video))
}

async fn acquire_flow_turn<'a>(
    gate: &'a FlowMediaGate,
    limit: usize,
    token: &tokio_util::sync::CancellationToken,
) -> Result<FlowMediaTurn<'a>, String> {
    loop {
        if token.is_cancelled() {
            return Err("Pembuatan media dibatalkan".into());
        }
        let changed = gate.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        {
            let mut active = gate.active.lock().unwrap();
            if *active < limit.max(1) {
                *active += 1;
                return Ok(FlowMediaTurn(gate));
            }
        }
        tokio::select! {
            biased;
            _ = token.cancelled() => {
                return Err("Pembuatan media dibatalkan".into());
            },
            _ = changed => {},
        }
    }
}

fn save_asset_file(project_id: &str, folder: &str, ext: &str, bytes: &[u8]) -> Result<String, String> {
    let rel = format!("{}/{}{}", folder, Uuid::new_v4(), ext);
    let abs = asset_abs_path(project_id, &rel);
    if let Some(parent) = abs.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&abs, bytes).map_err(|e| e.to_string())?;
    Ok(rel)
}

async fn finish_higgsfield(
    ctx: &TaskCtx,
    db: &DbPool,
    app: &AppHandle,
    key: &str,
    project_id: &str,
    clip_id: Option<&str>,
    character_id: Option<&str>,
    request_id: &str,
    target: &str,
    model: &str,
    prompt: &str,
) -> Result<Option<String>, String> {
    let is_video = target == "clip-video";
    let what = if is_video {
        "Membuat video"
    } else if target == "sheet" {
        "Membuat lembar karakter"
    } else {
        "Membuat gambar"
    };

    let result = hf::wait_for_result(key, request_id, |status, elapsed| {
        if status == "queued" {
            ctx.progress(0.12, Some("Mengantre di Higgsfield"));
        } else {
            let p = 0.15 + 0.75 * (1.0 - (-(elapsed as f64) / 45_000.0).exp());
            ctx.progress(p, Some(what));
        }
    })
    .await?;

    save_higgsfield_result(ctx, db, app, project_id, clip_id, character_id, request_id, target, model, prompt, "higgsfield", &result).await
}

async fn save_higgsfield_result(
    ctx: &TaskCtx, db: &DbPool, app: &AppHandle, project_id: &str,
    clip_id: Option<&str>, character_id: Option<&str>, request_id: &str,
    target: &str, model: &str, prompt: &str, provider: &str, result: &Value,
) -> Result<Option<String>, String> {
    if ctx.token.is_cancelled() { return Err("Pembuatan media dibatalkan".into()); }
    let is_video = target == "clip-video";
    let url = if is_video {
        result.get("video").and_then(|v| v.get("url")).and_then(|x| x.as_str())
    } else {
        result.get("images").and_then(|arr| arr.get(0)).and_then(|img| img.get("url")).and_then(|x| x.as_str())
    }
    .ok_or("Higgsfield selesai tanpa file hasil")?;

    ctx.progress(0.93, Some("Mengunduh hasil"));
    let (bytes, ctype) = download_to(url).await?;

    let ext = if ctype.contains("mp4") || url.ends_with(".mp4") {
        ".mp4"
    } else if ctype.contains("webp") || url.ends_with(".webp") {
        ".webp"
    } else if ctype.contains("jpeg") || ctype.contains("jpg") || url.ends_with(".jpg") {
        ".jpg"
    } else {
        ".png"
    };

    let rel = save_asset_file(project_id, if is_video { "videos" } else { "images" }, ext, &bytes)?;
    let abs = asset_abs_path(project_id, &rel);

    let (w, h) = probe_size(&abs).await.unwrap_or((1920, 1080));
    let dur_ms = if is_video {
        probe_duration_ms(&abs).await.ok().map(|d| d as i64)
    } else {
        None
    };

    let new_asset = NewAsset {
        project_id: project_id.to_string(),
        clip_id: clip_id.map(|s| s.to_string()),
        character_id: character_id.map(|s| s.to_string()),
        kind: if is_video { "video" } else if target == "thumbnail" { "thumbnail" } else { "image" }.to_string(),
        provider: provider.to_string(),
        model: Some(model.to_string()),
        prompt: Some(prompt.to_string()),
        remote_id: Some(request_id.to_string()),
        remote_url: Some(url.to_string()),
        local_path: rel,
        duration_ms: dur_ms,
        width: Some(w as i64),
        height: Some(h as i64),
        meta: json!({}),
    };

    let asset_row = {
        let conn = db.lock().unwrap();
        insert_asset(&conn, new_asset)?
    };

    let _ = app.emit("event:asset", &asset_row);

    if target == "sheet" {
        if let Some(cid) = character_id {
            let conn = db.lock().unwrap();
            let _ = set_character_sheet(&conn, cid, &asset_row.id);
            let _ = app.emit(
                "event:character",
                json!({
                    "characterId": cid,
                    "patch": { "sheetAssetId": asset_row.id }
                }),
            );
        }
    } else if target == "clip-image" {
        if let Some(cid) = clip_id {
            let conn = db.lock().unwrap();
            let _ = patch_clip(&conn, cid, json!({ "imageAssetId": asset_row.id }));
            let _ = app.emit(
                "event:clip",
                json!({
                    "clipId": cid,
                    "patch": { "imageAssetId": asset_row.id }
                }),
            );
            if let Some(p) = get_project(&conn, project_id) {
                let first = clips_of(&conn, project_id).into_iter().next();
                if p.cover_asset_id.is_none() || first.map(|c| c.id) == Some(cid.to_string()) {
                    let _ = touch_project(&conn, project_id, None);
                }
            }
        }
    } else if target == "clip-video" {
        if let Some(cid) = clip_id {
            let conn = db.lock().unwrap();
            let _ = patch_clip(
                &conn,
                cid,
                json!({ "videoAssetId": asset_row.id, "motionType": "video" }),
            );
            let _ = app.emit(
                "event:clip",
                json!({
                    "clipId": cid,
                    "patch": { "videoAssetId": asset_row.id, "motionType": "video" }
                }),
            );
        }
    }

    {
        let conn = db.lock().unwrap();
        let _ = touch_project(&conn, project_id, None);
    }

    Ok(Some(asset_row.id))
}

async fn finish_mcp(
    ctx: &TaskCtx, db: &DbPool, app: &AppHandle, project_id: &str,
    clip_id: Option<&str>, character_id: Option<&str>, target: &str,
    model: &HfModel, prompt: &str, body: &Value, references: &[std::path::PathBuf],
) -> Result<Option<String>, String> {
    let mut session = mcp::Session::new(db).await?;
    let balance = session.tool("balance", json!({})).await?;
    if balance["free_trial"]["unlim_available"].as_bool() != Some(true) {
        return Err("Unlimited MCP belum aktif atau sudah habis. Periksa trial di akun Higgsfield. Kredit tidak dipakai.".into());
    }
    let mut refs = Vec::new();
    for path in references {
        ctx.progress(0.08, Some("Mengunggah referensi ke Higgsfield MCP"));
        let id = tokio::select! {
            _ = ctx.token.cancelled() => return Err("Pembuatan media dibatalkan".into()),
            result = session.upload(path) => result?,
        };
        refs.push(id);
    }
    if ctx.token.is_cancelled() { return Err("Pembuatan media dibatalkan".into()); }
    let video = target == "clip-video";
    ctx.progress(0.12, Some("Mengirim ke Higgsfield MCP · Unlimited"));
    // Do not retry a submission: a lost response can still have created a remote job.
    let request_id = session.submit(if video { "video" } else { "image" }, &model.id, body, &refs,
        if video && model.input.mode == "first-frame" { "start_image" } else { "image" }).await?;
    ctx.set_remote(&request_id);
    {
        let conn = db.lock().unwrap();
        update_job(&conn, &ctx.job_id, json!({"payload":{"target":target,"model":model.id,"prompt":prompt}}))?;
    }
    let started = std::time::Instant::now();
    loop {
        if started.elapsed() > std::time::Duration::from_secs(20 * 60) {
            return Err("Higgsfield MCP belum selesai setelah 20 menit. Periksa Assets Higgsfield sebelum mengulang.".into());
        }
        let result = tokio::select! {
            _ = ctx.token.cancelled() => return Err("Pembuatan media dibatalkan; pekerjaan yang sudah dikirim dapat tetap selesai di Higgsfield.".into()),
            result = session.tool("jobs_wait", json!({"jobs":[{"index":0,"job_id":request_id}],"timeout_seconds":15})) => result?,
        };
        let job = &result["jobs"][0];
        match job["status"].as_str().unwrap_or("") {
            "completed" => {
                let url = job["result_url"].as_str().filter(|u| u.starts_with("https://")).ok_or("MCP selesai tanpa URL hasil HTTPS")?;
                let value = if video { json!({"video":{"url":url}}) } else { json!({"images":[{"url":url}]}) };
                return save_higgsfield_result(ctx, db, app, project_id, clip_id, character_id, &request_id,
                    target, &model.id, prompt, "higgsfield-mcp", &value).await;
            }
            "failed" | "canceled" | "nsfw" | "ip_detected" => {
                return Err(format!("Higgsfield MCP gagal: {}", job["error"].as_str().or(job["status"].as_str()).unwrap_or("tanpa keterangan")));
            }
            "lookup_failed" if job["retryable"].as_bool() != Some(true) => {
                return Err("Status generasi MCP tidak ditemukan. Periksa Assets Higgsfield sebelum mengulang.".into());
            }
            _ => ctx.progress(0.15 + 0.7 * (1.0 - (-(started.elapsed().as_millis() as f64) / 60000.0).exp()), Some("Menunggu hasil Higgsfield MCP")),
        }
        tokio::select! {
            _ = ctx.token.cancelled() => return Err("Pembuatan media dibatalkan".into()),
            _ = tokio::time::sleep(std::time::Duration::from_secs(result["poll_after_seconds"].as_u64().unwrap_or(3).clamp(1,15))) => {},
        }
    }
}

pub fn generate_clip_image(db: DbPool, job_mgr: Arc<JobManager>, app: AppHandle, clip_id: String) -> Result<Value,String> {
    start_image(db, job_mgr, app, clip_id, false)
}

pub fn generate_thumbnail(db: DbPool, job_mgr: Arc<JobManager>, app: AppHandle, project_id: String) -> Result<Value,String> {
    let clip_id = {
        let conn = db.lock().unwrap();
        let project = get_project(&conn, &project_id).ok_or("Proyek tidak ditemukan")?;
        if project.aspect_ratio != "16:9" { return Err("Thumbnail hanya untuk proyek YouTube 16:9".into()); }
        clips_of(&conn, &project_id).first().map(|c|c.id.clone()).ok_or("Susun naskah terlebih dahulu")?
    };
    start_image(db, job_mgr, app, clip_id, true)
}

fn start_image(
    db: DbPool,
    job_mgr: Arc<JobManager>,
    app: AppHandle,
    clip_id: String,
    thumbnail: bool,
) -> Result<Value, String> {
    let (project_id, clip, is_flow, job) = {
        let conn = db.lock().unwrap();
        let clip = get_clip(&conn, &clip_id).ok_or("Klip tidak ditemukan")?;
        let active = if thumbnail {
            crate::db::repo::active_project_job(&conn, &clip.project_id, "thumbnail")
        } else { active_job_for(&conn, Some(&clip_id), None, "image") };
        if let Some(job) = active { return Ok(job); }
        let project = get_project(&conn, &clip.project_id).ok_or("Proyek tidak ditemukan")?;
        if thumbnail && project.aspect_ratio != "16:9" { return Err("Thumbnail hanya untuk proyek YouTube 16:9".into()); }
        let model = get_image_model(project.image_model.as_deref());
        let is_flow = model.id.starts_with("flow:");
        let provider = if is_flow { "flow2api" } else if model.id.starts_with("vidgen:" ) { "vidgen" } else if model.id.starts_with("hfmcp:") { "higgsfield-mcp" } else { "higgsfield" };
        let job = insert_job(&conn, &clip.project_id, if thumbnail { "thumbnail" } else { "image" }, provider,
            json!({}), if thumbnail { None } else { Some(&clip_id) }, None)?;
        (clip.project_id.clone(), clip, is_flow, job)
    };

    let j_id = job.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let _ = app.emit("event:job", &job);

    let db_clone = db.clone();
    let app_clone = app.clone();
    let c_id = clip_id.clone();
    let p_id = project_id.clone();

    let vidgen = job["provider"] == "vidgen";
    job_mgr.spawn_task(&j_id, if vidgen { QueueName::Vidgen } else if is_flow { QueueName::Flow } else { QueueName::Higgsfield }, move |ctx| async move {
        ctx.progress(0.04, Some("Menyiapkan prompt"));
        let clip = if thumbnail { crate::services::thumbnail::prepare(&db_clone, &p_id, &ctx.token).await? } else { clip };
        let target = if thumbnail { "thumbnail" } else { "clip-image" };
        let output_clip = if thumbnail { None } else { Some(c_id.as_str()) };

        let (mode, flow_base, flow_key, key, project, model, chars) = {
            let conn = db_clone.lock().unwrap();
            let s = get_settings(&conn);
            let mode = s.get("flow2apiMode").and_then(|x| x.as_str()).unwrap_or("bundled").to_string();
            let fb = s.get("flow2apiBaseUrl").and_then(|x| x.as_str()).unwrap_or("http://localhost:8000").to_string();
            let fk = get_secret(&conn, "flow2api");
            let k = get_secret(&conn, "higgsfield").unwrap_or_default();
            let p = get_project(&conn, &p_id).ok_or("Proyek tidak ditemukan")?;
            let m = get_image_model(p.image_model.as_deref());
            let all_chars = characters_of(&conn, &p_id);
            let needed: Vec<CharacterRow> = all_chars
                .into_iter()
                .filter(|c| clip.character_ids.contains(&c.id))
                .collect();
            (mode, fb, fk, k, p, m, needed)
        };
        if thumbnail && project.aspect_ratio != "16:9" { return Err("Thumbnail hanya untuk proyek YouTube 16:9".into()); }
        let (flow_base, flow_key) = if model.id.starts_with("flow:") && crate::flow2api_runtime::is_bundled(&mode) {
            match flow2api_manager(&app_clone) {
                Some(manager) => crate::flow2api_runtime::resolve_target("bundled", &flow_base, None, Some(&manager)).await?,
                None => return Err("Flow2API internal belum siap. Buka Pengaturan lalu coba lagi.".into()),
            }
        } else {
            (flow_base, flow_key)
        };

        if model.id.starts_with("flow:") || model.id.starts_with("vidgen:") {
            let limit = max_refs(&model);
            let mut ref_images = Vec::new();
            let mut ref_names = Vec::new();

            if thumbnail && limit > 0 {
                let scene = { let conn=db_clone.lock().unwrap(); clip.image_asset_id.as_ref().and_then(|id|get_asset(&conn,id)) };
                if let Some(scene)=scene { if let Ok(bytes)=fs::read(asset_abs_path(&p_id,&scene.local_path)) { ref_images.push(bytes); } }
            }

            if (!thumbnail || ref_images.is_empty()) && limit > ref_images.len() && !chars.is_empty() {
                let c_ids: Vec<String> = chars.iter().map(|c| c.id.clone()).collect();
                wait_for_character_sheets(&db_clone, &c_ids, &ctx.token).await;

                for c in &chars {
                    if ref_images.len() >= limit {
                        break;
                    }
                    let sheet_path_opt = {
                        let conn = db_clone.lock().unwrap();
                        get_character(&conn, &c.id)
                            .and_then(|fresh| fresh.sheet_asset_id)
                            .and_then(|sid| get_asset(&conn, &sid))
                            .map(|a| asset_abs_path(&p_id, &a.local_path))
                    };
                    if let Some(path) = sheet_path_opt {
                        if let Ok(bytes) = fs::read(&path) {
                            ref_images.push(bytes);
                            ref_names.push(c.name.clone());
                        }
                    }
                }
            }

            let prompt = if thumbnail { clip.visual_prompt.chars().take(prompt_limit(&model)).collect::<String>() } else { clip_prompt(&project.aspect_ratio, styles::style_prompt(&project.style_id), &clip.visual_prompt, &chars, &ref_names, prompt_limit(&model)) };
            let img_bytes = if model.id.starts_with("vidgen:") {
                crate::services::vidgen::generate_image(&db_clone, &ctx, &model.id, &ref_images, &prompt, &project.aspect_ratio).await?.0
            } else {
                let _turn = acquire_flow_turn(flow_media_gate(), flow_parallel_accounts(&db_clone, &app_clone, false).await?, &ctx.token).await?;
                ctx.progress(0.16, Some("Mengirim ke Google Flow (Flow2API)"));
                crate::services::flow2api::generate_image(&flow_base, flow_key.as_deref(), &model.id, &prompt, &project.aspect_ratio, &ref_images, &ctx.token).await?
            };

            ctx.progress(0.92, Some("Menyimpan hasil gambar"));
            let ext = if img_bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
                ".jpg"
            } else if img_bytes.starts_with(b"RIFF") && img_bytes.len() > 12 && &img_bytes[8..12] == b"WEBP" {
                ".webp"
            } else {
                ".png"
            };
            let rel = save_asset_file(&p_id, "images", ext, &img_bytes)?;
            let abs = asset_abs_path(&p_id, &rel);
            let (w, h) = probe_size(&abs).await.unwrap_or((1920, 1080));

            let new_asset = NewAsset {
                project_id: p_id.clone(),
                clip_id: if thumbnail { None } else { Some(c_id.clone()) },
                character_id: None,
                kind: if thumbnail { "thumbnail" } else { "image" }.to_string(),
                provider: if model.id.starts_with("vidgen:") { "vidgen" } else { "flow2api" }.to_string(),
                model: Some(model.id.clone()),
                prompt: Some(prompt),
                remote_id: None,
                remote_url: None,
                local_path: rel,
                duration_ms: None,
                width: Some(w as i64),
                height: Some(h as i64),
                meta: json!({}),
            };

            let asset_row = {
                let conn = db_clone.lock().unwrap();
                insert_asset(&conn, new_asset)?
            };

            let _ = app_clone.emit("event:asset", &asset_row);

            if !thumbnail {
                let conn = db_clone.lock().unwrap();
                let _ = patch_clip(&conn, &c_id, json!({ "imageAssetId": asset_row.id }));
                let _ = app_clone.emit(
                    "event:clip",
                    json!({
                        "clipId": c_id,
                        "patch": { "imageAssetId": asset_row.id }
                    }),
                );
                if let Some(p) = get_project(&conn, &p_id) {
                    let first = clips_of(&conn, &p_id).into_iter().next();
                    if p.cover_asset_id.is_none() || first.map(|c| c.id) == Some(c_id.to_string()) {
                        let _ = touch_project(&conn, &p_id, None);
                    }
                }
                let _ = touch_project(&conn, &p_id, None);
            }

            return Ok(Some(asset_row.id));
        }

        if model.id.starts_with("hfmcp:") {
            let mut paths = Vec::new();
            let mut names = Vec::new();
            if thumbnail && max_refs(&model)>0 {
                let scene={let conn=db_clone.lock().unwrap();clip.image_asset_id.as_ref().and_then(|id|get_asset(&conn,id))};
                if let Some(scene)=scene { paths.push(asset_abs_path(&p_id,&scene.local_path)); }
            }
            let ids: Vec<String> = chars.iter().map(|c| c.id.clone()).collect();
            let use_sheets = !thumbnail || paths.is_empty();
            if use_sheets && max_refs(&model) > 0 { wait_for_character_sheets(&db_clone, &ids, &ctx.token).await; }
            {
                let conn = db_clone.lock().unwrap();
                for c in chars.iter().take(if use_sheets { max_refs(&model).saturating_sub(paths.len()) } else { 0 }) {
                    if let Some(asset) = get_character(&conn, &c.id).and_then(|c| c.sheet_asset_id).and_then(|id| get_asset(&conn, &id)) {
                        paths.push(asset_abs_path(&p_id, &asset.local_path));
                        names.push(c.name.clone());
                    }
                }
            }
            let prompt = if thumbnail { clip.visual_prompt.chars().take(prompt_limit(&model)).collect::<String>() } else { clip_prompt(&project.aspect_ratio, styles::style_prompt(&project.style_id), &clip.visual_prompt, &chars, &names, prompt_limit(&model)) };
            let body = image_body(&model, &prompt, &project.aspect_ratio, &[])?;
            return finish_mcp(&ctx, &db_clone, &app_clone, &p_id, output_clip, None, target, &model, &prompt, &body, &paths).await;
        }

        if key.is_empty() {
            return Err("Kunci Higgsfield belum diatur di Pengaturan.".into());
        }

        let scene=if thumbnail && max_refs(&model)>0 {let conn=db_clone.lock().unwrap();clip.image_asset_id.as_ref().and_then(|id|get_asset(&conn,id))} else {None};
        let (ref_urls, ref_names) = if let Some(scene)=scene {
            (vec![public_url_for(&key,&db_clone,&scene).await?], Vec::new())
        } else { reference_urls(&key, &db_clone, &model, &chars, &ctx.token).await };
        let prompt = if thumbnail { clip.visual_prompt.chars().take(prompt_limit(&model)).collect::<String>() } else { clip_prompt(&project.aspect_ratio, styles::style_prompt(&project.style_id), &clip.visual_prompt, &chars, &ref_names, prompt_limit(&model)) };
        let body = image_body(&model, &prompt, &project.aspect_ratio, &ref_urls)?;

        ctx.progress(0.08, Some("Mengirim ke Higgsfield"));
        let request_id = hf::submit(&key, &model.id, &body).await?;
        ctx.set_remote(&request_id);

        let payload = json!({
            "target": target,
            "model": model.id,
            "prompt": prompt
        });
        {
            let conn = db_clone.lock().unwrap();
            let _ = update_job(&conn, &ctx.job_id, json!({ "payload": payload }));
        }

        finish_higgsfield(
            &ctx,
            &db_clone,
            &app_clone,
            &key,
            &p_id,
            output_clip,
            None,
            &request_id,
            target,
            &model.id,
            &prompt,
        )
        .await
    });

    Ok(job)
}

pub fn generate_character_sheet(
    db: DbPool,
    job_mgr: Arc<JobManager>,
    app: AppHandle,
    character_id: String,
) -> Result<Value, String> {
    let (project_id, character, is_flow) = {
        let conn = db.lock().unwrap();
        if let Some(j) = active_job_for(&conn, None, Some(&character_id), "sheet") {
            return Ok(j);
        }
        let c = get_character(&conn, &character_id).ok_or("Karakter tidak ditemukan")?;
        let p = get_project(&conn, &c.project_id).ok_or("Proyek tidak ditemukan")?;
        let m = get_image_model(p.image_model.as_deref());
        let is_flow = m.id.starts_with("flow:");
        (c.project_id.clone(), c, is_flow)
    };

    let provider = if is_flow { "flow2api" } else {
        let conn = db.lock().unwrap();
        let model = get_project(&conn, &project_id).and_then(|p| p.image_model).unwrap_or_default();
        if model.starts_with("vidgen:") { "vidgen" } else if model.starts_with("hfmcp:") { "higgsfield-mcp" } else { "higgsfield" }
    };
    let job = {
        let conn = db.lock().unwrap();
        insert_job(&conn, &project_id, "sheet", provider, json!({}), None, Some(&character_id))?
    };

    let j_id = job.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let _ = app.emit("event:job", &job);

    let db_clone = db.clone();
    let app_clone = app.clone();
    let char_id = character_id.clone();
    let p_id = project_id.clone();

    job_mgr.spawn_task(&j_id, if provider == "vidgen" { QueueName::Vidgen } else if is_flow { QueueName::Flow } else { QueueName::Sheet }, move |ctx| async move {
        ctx.progress(0.04, Some("Menyiapkan lembar karakter"));

        let (mode, flow_base, flow_key, key, model, style) = {
            let conn = db_clone.lock().unwrap();
            let s = get_settings(&conn);
            let mode = s.get("flow2apiMode").and_then(|x| x.as_str()).unwrap_or("bundled").to_string();
            let fb = s.get("flow2apiBaseUrl").and_then(|x| x.as_str()).unwrap_or("http://localhost:8000").to_string();
            let fk = get_secret(&conn, "flow2api");
            let k = get_secret(&conn, "higgsfield").unwrap_or_default();
            let p = get_project(&conn, &p_id).ok_or("Proyek tidak ditemukan")?;
            let m = get_image_model(p.image_model.as_deref());
            let style = (styles::style_prompt(&p.style_id).to_string(), styles::style_character_hint(&p.style_id).to_string());
            (mode, fb, fk, k, m, style)
        };
        let (flow_base, flow_key) = if model.id.starts_with("flow:") && crate::flow2api_runtime::is_bundled(&mode) {
            match flow2api_manager(&app_clone) {
                Some(manager) => crate::flow2api_runtime::resolve_target("bundled", &flow_base, None, Some(&manager)).await?,
                None => return Err("Flow2API internal belum siap. Buka Pengaturan lalu coba lagi.".into()),
            }
        } else {
            (flow_base, flow_key)
        };

        if model.id.starts_with("flow:") || model.id.starts_with("vidgen:") {
            let prompt = sheet_prompt(&character, &style.0, &style.1);
            let img_bytes = if model.id.starts_with("vidgen:") {
                crate::services::vidgen::generate_image(&db_clone, &ctx, &model.id, &[], &prompt, "16:9").await?.0
            } else {
                let _turn = acquire_flow_turn(flow_media_gate(), flow_parallel_accounts(&db_clone, &app_clone, false).await?, &ctx.token).await?;
                ctx.progress(0.16, Some("Mengirim ke Google Flow (Flow2API)"));
                crate::services::flow2api::generate_image(&flow_base, flow_key.as_deref(), &model.id, &prompt, "16:9", &[], &ctx.token).await?
            };

            ctx.progress(0.92, Some("Menyimpan lembar karakter"));
            let ext = if img_bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
                ".jpg"
            } else if img_bytes.starts_with(b"RIFF") && img_bytes.len() > 12 && &img_bytes[8..12] == b"WEBP" {
                ".webp"
            } else {
                ".png"
            };
            let rel = save_asset_file(&p_id, "images", ext, &img_bytes)?;
            let abs = asset_abs_path(&p_id, &rel);
            let (w, h) = probe_size(&abs).await.unwrap_or((1920, 1080));

            let new_asset = NewAsset {
                project_id: p_id.clone(),
                clip_id: None,
                character_id: Some(char_id.clone()),
                kind: "image".to_string(),
                provider: if model.id.starts_with("vidgen:") { "vidgen" } else { "flow2api" }.to_string(),
                model: Some(model.id.clone()),
                prompt: Some(prompt),
                remote_id: None,
                remote_url: None,
                local_path: rel,
                duration_ms: None,
                width: Some(w as i64),
                height: Some(h as i64),
                meta: json!({}),
            };

            let asset_row = {
                let conn = db_clone.lock().unwrap();
                insert_asset(&conn, new_asset)?
            };

            let _ = app_clone.emit("event:asset", &asset_row);

            {
                let conn = db_clone.lock().unwrap();
                let _ = set_character_sheet(&conn, &char_id, &asset_row.id);
                let _ = app_clone.emit(
                    "event:character",
                    json!({
                        "characterId": char_id,
                        "patch": { "sheetAssetId": asset_row.id }
                    }),
                );
                let _ = touch_project(&conn, &p_id, None);
            }

            return Ok(Some(asset_row.id));
        }

        if model.id.starts_with("hfmcp:") {
            let prompt = sheet_prompt(&character, &style.0, &style.1);
            let body = image_body(&model, &prompt, "16:9", &[])?;
            return finish_mcp(&ctx, &db_clone, &app_clone, &p_id, None, Some(&char_id), "sheet", &model, &prompt, &body, &[]).await;
        }

        if key.is_empty() {
            return Err("Kunci Higgsfield belum diatur di Pengaturan.".into());
        }

        let prompt = sheet_prompt(&character, &style.0, &style.1);
        let body = image_body(&model, &prompt, "16:9", &[])?;

        ctx.progress(0.08, Some("Mengirim ke Higgsfield"));
        let request_id = hf::submit(&key, &model.id, &body).await?;
        ctx.set_remote(&request_id);

        let payload = json!({
            "target": "sheet",
            "model": model.id,
            "prompt": prompt
        });
        {
            let conn = db_clone.lock().unwrap();
            let _ = update_job(&conn, &ctx.job_id, json!({ "payload": payload }));
        }

        finish_higgsfield(
            &ctx,
            &db_clone,
            &app_clone,
            &key,
            &p_id,
            None,
            Some(&char_id),
            &request_id,
            "sheet",
            &model.id,
            &prompt,
        )
        .await
    });

    Ok(job)
}

pub(crate) async fn store_video_bytes(ctx: &TaskCtx, db_clone: DbPool, app_clone: AppHandle, p_id: String, c_id: String,
    model: HfModel, base_prompt: String, provider: &str, video_bytes: Vec<u8>, meta: Value) -> Result<Option<String>, String> {
    if ctx.token.is_cancelled() { return Err("Dibatalkan".into()); }
    ctx.progress(0.92, Some("Menyimpan hasil video"));
    let ext = if video_bytes.starts_with(b"GIF8") {
        ".gif"
    } else if video_bytes.starts_with(b"\x1a\x45\xdf\xa3") {
        ".webm"
    } else {
        ".mp4"
    };
    let rel = save_asset_file(&p_id, "videos", ext, &video_bytes)?;
    let abs = asset_abs_path(&p_id, &rel);
    let (w, h) = probe_size(&abs).await.unwrap_or((1920, 1080));
    let dur_ms = probe_duration_ms(&abs).await.ok().map(|d| d as i64);

    let new_asset = NewAsset {
        project_id: p_id.clone(),
        clip_id: Some(c_id.clone()),
        character_id: None,
        kind: "video".to_string(),
        provider: provider.to_string(),
        model: Some(model.id.clone()),
        prompt: Some(base_prompt),
        remote_id: meta["remoteId"].as_str().map(String::from),
        remote_url: None,
        local_path: rel,
        duration_ms: dur_ms,
        width: Some(w as i64),
        height: Some(h as i64),
        meta,
    };

    let asset_row = {
        let conn = db_clone.lock().unwrap();
        insert_asset(&conn, new_asset)?
    };

    let _ = app_clone.emit("event:asset", &asset_row);

    {
        let conn = db_clone.lock().unwrap();
        let _ = patch_clip(
            &conn,
            &c_id,
            json!({ "videoAssetId": asset_row.id, "motionType": "video" }),
        );
        let _ = app_clone.emit(
            "event:clip",
            json!({
                "clipId": c_id,
                "patch": { "videoAssetId": asset_row.id, "motionType": "video" }
            }),
        );
        let _ = touch_project(&conn, &p_id, None);
    }

    return Ok(Some(asset_row.id));
}

pub fn generate_clip_video(
    db: DbPool,
    job_mgr: Arc<JobManager>,
    app: AppHandle,
    clip_id: String,
) -> Result<Value, String> {
    let (project_id, clip, is_flow) = {
        let conn = db.lock().unwrap();
        if let Some(j) = active_job_for(&conn, Some(&clip_id), None, "video") {
            return Ok(j);
        }
        let c = get_clip(&conn, &clip_id).ok_or("Klip tidak ditemukan")?;
        if c.image_asset_id.is_none() {
            return Err("Buat gambar klip ini dulu sebelum membuat video AI.".into());
        }
        let p = get_project(&conn, &c.project_id).ok_or("Proyek tidak ditemukan")?;
        let m = get_video_model(p.video_model.as_deref());
        let is_flow = m.id.starts_with("flow:");
        (c.project_id.clone(), c, is_flow)
    };

    let provider = if is_flow { "flow2api" } else {
        let conn = db.lock().unwrap();
        let selected = get_project(&conn, &project_id).and_then(|p| p.video_model).unwrap_or_default();
        if selected.starts_with("vidgen:") { "vidgen" } else if selected.starts_with("hfmcp:") { "higgsfield-mcp" } else { "higgsfield" }
    };
    if provider == "vidgen" && !crate::services::vidgen::list_accounts(&db)?.iter().any(|a|a["enabled"]==true && a["configured"]==true) {
        return Err("Tambahkan API key dan aktifkan akun di Pengaturan → Layanan AI → Vidgen".into());
    }
    let job = {
        let conn = db.lock().unwrap();
        insert_job(&conn, &project_id, "video", provider, json!({}), Some(&clip_id), None)?
    };

    let j_id = job.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let _ = app.emit("event:job", &job);

    let db_clone = db.clone();
    let app_clone = app.clone();
    let c_id = clip_id.clone();
    let p_id = project_id.clone();

    job_mgr.spawn_task(&j_id, if provider == "vidgen" { QueueName::Vidgen } else if is_flow { QueueName::Flow } else { QueueName::Higgsfield }, move |ctx| async move {
        ctx.progress(0.05, Some("Menyiapkan gambar awal"));

        let (mode, flow_base, flow_key, key, project, model, image_asset) = {
            let conn = db_clone.lock().unwrap();
            let s = get_settings(&conn);
            let mode = s.get("flow2apiMode").and_then(|x| x.as_str()).unwrap_or("bundled").to_string();
            let fb = s.get("flow2apiBaseUrl").and_then(|x| x.as_str()).unwrap_or("http://localhost:8000").to_string();
            let fk = get_secret(&conn, "flow2api");
            let k = get_secret(&conn, "higgsfield").unwrap_or_default();
            let p = get_project(&conn, &p_id).ok_or("Proyek tidak ditemukan")?;
            let m = get_video_model(p.video_model.as_deref());
            let img = clip.image_asset_id.as_ref().and_then(|id| get_asset(&conn, id)).ok_or("Gambar klip tidak ditemukan")?;
            (mode, fb, fk, k, p, m, img)
        };
        let (flow_base, flow_key) = if model.id.starts_with("flow:") && crate::flow2api_runtime::is_bundled(&mode) {
            match flow2api_manager(&app_clone) {
                Some(manager) => crate::flow2api_runtime::resolve_target("bundled", &flow_base, None, Some(&manager)).await?,
                None => return Err("Flow2API internal belum siap. Buka Pengaturan lalu coba lagi.".into()),
            }
        } else {
            (flow_base, flow_key)
        };

        if model.id.starts_with("vidgen:") {
            let prompt = if clip.video_prompt.trim().is_empty() { split_avoid(&clip.visual_prompt).0 } else { clip.video_prompt.clone() };
            let prompt = format!("Animate the reference image, preserving its characters and visual style. {prompt}. Silent clip, no music or voice.");
            let path = asset_abs_path(&p_id, &image_asset.local_path);
            let (bytes, meta) = crate::services::vidgen::generate_video(&db_clone, &ctx, &path, &prompt, &project.aspect_ratio, project.video_resolution.as_deref()).await?;
            return store_video_bytes(&ctx, db_clone, app_clone, p_id, c_id, model, prompt, "vidgen", bytes, meta).await;
        }

        if model.id.starts_with("flow:") {
            let frame_path = asset_abs_path(&p_id, &image_asset.local_path);
            let frame_bytes = fs::read(&frame_path)
                .map_err(|e| format!("Gagal membaca frame gambar {}: {e}", frame_path.display()))?;

            let (scene, _avoid) = split_avoid(&clip.visual_prompt);
            let base_prompt = if !clip.video_prompt.trim().is_empty() {
                clip.video_prompt.trim().to_string()
            } else {
                format!("{}. Subtle natural motion of the characters and environment, the camera slowly pushes in.", scene)
            };

            let sec = (clip.duration_ms as f64) / 1000.0;
            ctx.progress(0.11, Some("Menunggu antrean media Flow"));
            let _turn = acquire_flow_turn(flow_media_gate(), flow_parallel_accounts(&db_clone, &app_clone, true).await?, &ctx.token).await?;
            ctx.progress(0.12, Some("Mengirim ke Google Flow (Flow2API)"));

            let video_bytes = crate::services::flow2api::generate_video(
                &flow_base,
                flow_key.as_deref(),
                &model.id,
                &base_prompt,
                &frame_bytes,
                sec,
                &project.aspect_ratio,
                &ctx.token,
            ).await?;

            return store_video_bytes(&ctx, db_clone, app_clone, p_id, c_id, model, base_prompt, "flow2api", video_bytes, json!({})).await;
        }

        if model.id.starts_with("hfmcp:") {
            let (scene, _avoid) = split_avoid(&clip.visual_prompt);
            let base_prompt = if !clip.video_prompt.trim().is_empty() {
                clip.video_prompt.trim().to_string()
            } else {
                format!("{}. Subtle natural motion of the characters and environment, the camera slowly pushes in.", scene)
            };
            let prompt = if model.input.mode == "reference" {
                format!("Open on the exact scene of the reference image and keep its art style, characters and composition. {base_prompt}")
            } else { base_prompt };
            let body = video_body(&model, &prompt, "", (clip.duration_ms as f64) / 1000.0, &project.aspect_ratio, project.video_resolution.as_deref())?;
            let paths = [asset_abs_path(&p_id, &image_asset.local_path)];
            return finish_mcp(&ctx, &db_clone, &app_clone, &p_id, Some(&c_id), None, "clip-video", &model, &prompt, &body, &paths).await;
        }

        if key.is_empty() {
            return Err("Kunci Higgsfield belum diatur di Pengaturan.".into());
        }

        let image_url = public_url_for(&key, &db_clone, &image_asset).await?;

        let (scene, _avoid) = split_avoid(&clip.visual_prompt);
        let base_prompt = if !clip.video_prompt.trim().is_empty() {
            clip.video_prompt.trim().to_string()
        } else {
            format!("{}. Subtle natural motion of the characters and environment, the camera slowly pushes in.", scene)
        };

        let prompt = if model.input.mode == "reference" {
            format!("Open on the exact scene of the reference image and keep its art style, characters and composition. {base_prompt}")
        } else {
            base_prompt
        };

        let sec = (clip.duration_ms as f64) / 1000.0;
        let body = video_body(&model, &prompt, &image_url, sec, &project.aspect_ratio, project.video_resolution.as_deref())?;

        ctx.progress(0.08, Some("Mengirim ke Higgsfield"));
        let request_id = hf::submit(&key, &model.id, &body).await?;
        ctx.set_remote(&request_id);

        let payload = json!({
            "target": "clip-video",
            "model": model.id,
            "prompt": prompt
        });
        {
            let conn = db_clone.lock().unwrap();
            let _ = update_job(&conn, &ctx.job_id, json!({ "payload": payload }));
        }

        finish_higgsfield(
            &ctx,
            &db_clone,
            &app_clone,
            &key,
            &p_id,
            Some(&c_id),
            None,
            &request_id,
            "clip-video",
            &model.id,
            &prompt,
        )
        .await
    });

    Ok(job)
}

pub fn generate_clip_tts(
    db: DbPool,
    job_mgr: Arc<JobManager>,
    app: AppHandle,
    clip_id: String,
) -> Result<Value, String> {
    let (project_id, clip) = {
        let conn = db.lock().unwrap();
        if let Some(j) = active_job_for(&conn, Some(&clip_id), None, "tts") {
            return Ok(j);
        }
        let c = get_clip(&conn, &clip_id).ok_or("Klip tidak ditemukan")?;
        (c.project_id.clone(), c)
    };

    let text = clip.narration.trim().to_string();
    if text.is_empty() {
        return Err("Naskah klip ini masih kosong".into());
    }

    let project = {
        let conn = db.lock().unwrap();
        get_project(&conn, &project_id).ok_or("Proyek tidak ditemukan")?
    };

    let job = {
        let conn = db.lock().unwrap();
        insert_job(&conn, &project_id, "tts", &project.tts_provider, json!({}), Some(&clip_id), None)?
    };

    let j_id = job.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let _ = app.emit("event:job", &job);

    let db_clone = db.clone();
    let app_clone = app.clone();
    let c_id = clip_id.clone();
    let p_id = project_id.clone();

    job_mgr.spawn_task(&j_id, QueueName::Tts, move |ctx| async move {
        let (provider, voice, lang, model, key) = {
            let conn = db_clone.lock().unwrap();
            let p = get_project(&conn, &p_id).ok_or("Proyek tidak ditemukan")?;
            let s = get_settings(&conn);
            let prov = p.tts_provider.clone();
            let k = get_secret(&conn, &prov).unwrap_or_default();
            let m = if prov == "gemini" {
                // No fallback model here: an empty value is reported to the user
                // by gemini::speak_with_model instead of silently swapping in a
                // different model than the one chosen in Pengaturan.
                s.get("geminiTtsModel").and_then(|x| x.as_str()).unwrap_or("").to_string()
            } else {
                s.get("elevenModel").and_then(|x| x.as_str()).unwrap_or("eleven_multilingual_v2").to_string()
            };
            let v = if !p.tts_voice.is_empty() {
                p.tts_voice.clone()
            } else if prov == "gemini" {
                "Charon".to_string()
            } else {
                String::new()
            };
            (prov, v, p.language.clone(), m, k)
        };

        ctx.progress(0.2, Some("Membuat suara"));

        let (rel, words_opt, estimated) = if provider == "elevenlabs" {
            if voice.is_empty() {
                return Err("Pilih suara ElevenLabs dulu di langkah Ide cerita.".into());
            }
            let (audio, words) = elevenlabs::speak(&key, &voice, &model, &text, &lang).await?;
            let r = save_asset_file(&p_id, "audio", ".mp3", &audio)?;
            (r, Some(words), false)
        } else {
            let wav = gemini::speak_with_model(&key, &model, &text, &voice, &lang).await?;
            let r = save_asset_file(&p_id, "audio", ".wav", &wav)?;
            (r, None, true)
        };

        ctx.progress(0.85, Some("Mengukur durasi"));
        let abs = asset_abs_path(&p_id, &rel);
        let audio_ms = probe_duration_ms(&abs).await.unwrap_or(2000.0) as i64;

        let words = if let Some(w) = words_opt {
            w
        } else {
            estimate_word_timings(&strip_voice_tags(&text), (audio_ms as f64) / 1000.0)
        };

        let meta = json!({
            "words": words,
            "estimatedTimings": estimated,
            "voice": voice
        });

        let new_asset = NewAsset {
            project_id: p_id.clone(),
            clip_id: Some(c_id.clone()),
            character_id: None,
            kind: "audio".to_string(),
            provider: provider.clone(),
            model: Some(model),
            prompt: Some(text),
            remote_id: None,
            remote_url: None,
            local_path: rel,
            duration_ms: Some(audio_ms),
            width: None,
            height: None,
            meta,
        };

        let asset_row = {
            let conn = db_clone.lock().unwrap();
            insert_asset(&conn, new_asset)?
        };

        let _ = app_clone.emit("event:asset", &asset_row);

        let dur = (audio_ms + 400).max(1500);
        {
            let conn = db_clone.lock().unwrap();
            let _ = patch_clip(
                &conn,
                &c_id,
                json!({ "audioAssetId": asset_row.id, "durationMs": dur }),
            );
            let _ = touch_project(&conn, &p_id, None);
        }

        let _ = app_clone.emit(
            "event:clip",
            json!({
                "clipId": c_id,
                "patch": { "audioAssetId": asset_row.id, "durationMs": dur }
            }),
        );

        Ok(Some(asset_row.id))
    });

    Ok(job)
}

pub fn generate_missing_images(
    db: DbPool,
    job_mgr: Arc<JobManager>,
    app: AppHandle,
    project_id: String,
) -> Result<Vec<Value>, String> {
    let mut jobs = Vec::new();
    let (project, clips, chars) = {
        let conn = db.lock().unwrap();
        let p = get_project(&conn, &project_id).ok_or("Proyek tidak ditemukan")?;
        let cl = clips_of(&conn, &project_id);
        let ch = characters_of(&conn, &project_id);
        (p, cl, ch)
    };

    let model = get_image_model(project.image_model.as_deref());
    if max_refs(&model) > 0 {
        let needed_char_ids: std::collections::HashSet<String> = clips.iter().flat_map(|c| c.character_ids.clone()).collect();
        for c in chars {
            if needed_char_ids.contains(&c.id) && c.sheet_asset_id.is_none() {
                if let Ok(j) = generate_character_sheet(db.clone(), job_mgr.clone(), app.clone(), c.id) {
                    jobs.push(j);
                }
            }
        }
    }

    for c in clips {
        if c.image_asset_id.is_none() {
            if let Ok(j) = generate_clip_image(db.clone(), job_mgr.clone(), app.clone(), c.id) {
                jobs.push(j);
            }
        }
    }

    Ok(jobs)
}

pub fn generate_missing_videos(
    db: DbPool,
    job_mgr: Arc<JobManager>,
    app: AppHandle,
    project_id: String,
) -> Result<Vec<Value>, String> {
    let clips = {
        let conn = db.lock().unwrap();
        clips_of(&conn, &project_id)
    };

    if clips.iter().any(|c| c.image_asset_id.is_none()) {
        return Err("Buat gambar semua klip dulu sebelum menjadikannya video.".into());
    }

    let mut jobs = Vec::new();
    for c in clips {
        if c.video_asset_id.is_none() {
            if let Ok(j) = generate_clip_video(db.clone(), job_mgr.clone(), app.clone(), c.id) {
                jobs.push(j);
            }
        }
    }

    Ok(jobs)
}

pub fn generate_missing_tts(
    db: DbPool,
    job_mgr: Arc<JobManager>,
    app: AppHandle,
    project_id: String,
) -> Result<Vec<Value>, String> {
    let clips = {
        let conn = db.lock().unwrap();
        clips_of(&conn, &project_id)
    };

    let mut jobs = Vec::new();
    for c in clips {
        if c.audio_asset_id.is_none() && !c.narration.trim().is_empty() {
            if let Ok(j) = generate_clip_tts(db.clone(), job_mgr.clone(), app.clone(), c.id) {
                jobs.push(j);
            }
        }
    }

    Ok(jobs)
}

pub fn set_active_asset(
    db: &DbPool,
    app: &AppHandle,
    clip_id: &str,
    asset_id: &str,
) -> Result<(), String> {
    let (kind, duration_ms) = {
        let conn = db.lock().unwrap();
        let asset = get_asset(&conn, asset_id).ok_or("Aset tidak ditemukan")?;
        if asset.clip_id.as_deref() != Some(clip_id) {
            return Err("Aset tidak cocok dengan klip ini".into());
        }
        (asset.kind, asset.duration_ms)
    };

    let patch = match kind.as_str() {
        "image" => json!({ "imageAssetId": asset_id }),
        "video" => json!({ "videoAssetId": asset_id }),
        "audio" => {
            let dur = (duration_ms.unwrap_or(0) + 400).max(1500);
            json!({ "audioAssetId": asset_id, "durationMs": dur })
        }
        _ => json!({}),
    };

    {
        let conn = db.lock().unwrap();
        patch_clip(&conn, clip_id, patch.clone())?;
    }

    let _ = app.emit(
        "event:clip",
        json!({
            "clipId": clip_id,
            "patch": patch
        }),
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn media_gate_admits_more_than_three_accounts() {
        let gate = FlowMediaGate::default();
        let token = tokio_util::sync::CancellationToken::new();
        let mut turns = Vec::new();
        for _ in 0..25 { turns.push(acquire_flow_turn(&gate, 25, &token).await.unwrap()); }
        let next = acquire_flow_turn(&gate, 25, &token);
        tokio::pin!(next);
        assert!(tokio::time::timeout(std::time::Duration::from_millis(20), &mut next).await.is_err());
        turns.pop();
        assert!(tokio::time::timeout(std::time::Duration::from_secs(1), &mut next).await.is_ok());
    }

    #[tokio::test]
    async fn image_turn_serializes_until_the_previous_image_finishes() {
        let gate = FlowMediaGate::default();
        let token = tokio_util::sync::CancellationToken::new();
        let first = acquire_flow_turn(&gate, 1, &token).await.unwrap();
        let second = acquire_flow_turn(&gate, 1, &token);
        tokio::pin!(second);
        assert!(tokio::time::timeout(std::time::Duration::from_millis(30), &mut second).await.is_err());
        drop(first);
        assert!(tokio::time::timeout(std::time::Duration::from_secs(1), &mut second).await.is_ok());
    }

    #[tokio::test]
    async fn three_flow_accounts_run_together_and_lower_limits_wait_for_existing_work() {
        let gate = FlowMediaGate::default();
        let token = tokio_util::sync::CancellationToken::new();
        let first = acquire_flow_turn(&gate, 3, &token).await.unwrap();
        let second = acquire_flow_turn(&gate, 3, &token).await.unwrap();
        let third = acquire_flow_turn(&gate, 3, &token).await.unwrap();
        let fourth = acquire_flow_turn(&gate, 3, &token);
        tokio::pin!(fourth);
        assert!(tokio::time::timeout(std::time::Duration::from_millis(20), &mut fourth).await.is_err());
        drop(third);
        let fourth = fourth.await.unwrap();
        let single = acquire_flow_turn(&gate, 1, &token);
        tokio::pin!(single);
        drop(first);
        drop(second);
        assert!(tokio::time::timeout(std::time::Duration::from_millis(20), &mut single).await.is_err());
        drop(fourth);
        assert!(tokio::time::timeout(std::time::Duration::from_secs(1), &mut single).await.is_ok());
    }

    #[tokio::test]
    async fn canceled_image_turn_does_not_block_next_image() {
        let gate = FlowMediaGate::default();
        let first_token = tokio_util::sync::CancellationToken::new();
        let waiting_token = tokio_util::sync::CancellationToken::new();
        let first = acquire_flow_turn(&gate, 1, &first_token).await.unwrap();
        let waiting = acquire_flow_turn(&gate, 1, &waiting_token);
        tokio::pin!(waiting);
        assert!(tokio::time::timeout(std::time::Duration::from_millis(30), &mut waiting).await.is_err());
        waiting_token.cancel();
        assert!(waiting.await.is_err());
        drop(first);
        let next = acquire_flow_turn(&gate, 1, &first_token).await;
        assert!(next.is_ok());
    }

    #[test]
    fn clip_str_stops_before_a_multibyte_character() {
        assert_eq!(clip_str("ab😀cd", 3), "ab");
        assert_eq!(clip_str("ab😀cd", 2), "ab");
        assert_eq!(clip_str("éx", 1), "");
        assert_eq!(clip_str("abc", 0), "");
    }

    #[test]
    fn clip_prompt_leads_with_the_style() {
        let c = CharacterRow {
            id: "c1".into(),
            project_id: "p1".into(),
            name: "Maya".into(),
            description: "A young nurse. Default outfit: blue scrubs.".into(),
            sheet_asset_id: None,
            sort: 0,
        };
        let prompt = clip_prompt(
            "16:9",
            "High-quality anime film still, clean cel shading. No text.",
            "Setting: a hospital ward at dusk.\nAction: Maya checks a chart.",
            &[c],
            &[],
            4000,
        );
        assert!(prompt.starts_with("Art style (must match exactly): High-quality anime film still"));
        assert!(prompt.contains("Scene:\nSetting: a hospital ward"));
        assert!(prompt.ends_with("Frame format 16:9, full-bleed composition."));
    }

    #[test]
    fn clip_prompt_keeps_style_when_truncated() {
        let long_scene = "x".repeat(5000);
        let prompt = clip_prompt("16:9", "ANIME STYLE.", &long_scene, &[], &[], 400);
        assert!(prompt.starts_with("Art style (must match exactly): ANIME STYLE."));
        assert!(prompt.len() <= 400);
    }

    #[test]
    fn sheet_prompt_carries_the_style() {
        let c = CharacterRow {
            id: "c1".into(),
            project_id: "p1".into(),
            name: "Maya".into(),
            description: "A young nurse.".into(),
            sheet_asset_id: None,
            sort: 0,
        };
        let prompt = sheet_prompt(&c, "High-quality anime film still.", "an anime character with clean cel shading");
        assert!(prompt.contains("an anime character with clean cel shading"));
        assert!(prompt.contains("High-quality anime film still."));
        assert!(prompt.contains("Character reference sheet for \"Maya\""));
    }

    #[test]
    fn mcp_catalog_builds_image_and_video_requests_without_api_fallback() {
        let models: Vec<_> = get_catalog_models().into_iter().filter(|m| m.id.starts_with("hfmcp:")).collect();
        assert_eq!(models.len(), 68);
        for m in models {
            let body = if m.kind == "image" {
                image_body(&m, "Test prompt", "16:9", &[]).unwrap()
            } else {
                video_body(&m, "Test prompt", "", 7.0, "16:9", None).unwrap()
            };
            assert!(body["prompt"].as_str().unwrap().contains("Test prompt"));
            if let Some(mode) = body.get("mode") {
                assert!(m.props["mode"]["enum"].as_array().unwrap().contains(mode));
            }
            assert_ne!(body["mode"], "t2v");
            assert_ne!(body["mode"], "text-to-video");
        }
        assert!(get_image_model(Some("hfmcp:removed-model")).id.starts_with("hfmcp:"));
        assert!(get_video_model(Some("hfmcp:removed-model")).id.starts_with("hfmcp:"));
        assert!(!get_image_model(None).id.starts_with("hfmcp:"));
    }

    #[test]
    fn test_catalog_models_not_empty() {
        let models = get_catalog_models();
        println!("Loaded {} models from catalog", models.len());
        assert!(!models.is_empty(), "Catalog models should not be empty");
        let img = get_image_model(None);
        println!("Default image model: {}", img.id);
        let vid = get_video_model(None);
        println!("Default video model: {}", vid.id);
        let flow_img = get_image_model(Some("flow:gemini-3.1-flash-image"));
        println!("Flow image model: {}", flow_img.id);
        let flow_vid = get_video_model(Some("flow:veo-3.1-fast"));
        println!("Flow video model: {}", flow_vid.id);
        // Ids saved while the app used useapi.net still resolve to a real model.
        let aliased = get_image_model(Some("flow:nano-banana-2"));
        println!("Aliased flow image model: {}", aliased.id);
        assert_eq!(aliased.id, "flow:gemini-3.1-flash-image");
    }
}


pub async fn estimate_credits(
    db: &DbPool,
    kind: &str,
    model_id: &str,
    aspect: &str,
    seconds: Option<f64>,
    resolution: Option<&str>,
) -> Option<Value> {
    if model_id.starts_with("hfmcp:") { return None; }

    if model_id.starts_with("vidgen:") {
        let credits = if kind == "video" && model_id == "vidgen:gemini-omni-1.2-unlimited" { Some(0.0) } else { crate::services::vidgen::image_estimate(model_id, aspect) }?;
        return Some(json!({"credits":credits,"usd":null}));
    }
    if model_id.starts_with("flow:") {
        return Some(json!({
            "credits": 0,
            "usd": 0.0
        }));
    }

    let sec = seconds.unwrap_or(5.0);
    let (model, body) = if kind == "image" {
        let m = get_image_model(Some(model_id));
        let b = image_body(&m, "estimate", aspect, &[]).ok()?;
        (m, b)
    } else {
        let m = get_video_model(Some(model_id));
        let b = video_body(&m, "estimate", "https://example.com/frame.png", sec, aspect, resolution).ok()?;
        (m, b)
    };

    let cache_key = format!("estimate:{}:{body}", model.id);
    let hit = {
        let conn = db.lock().unwrap();
        read_cache(&conn, &cache_key)
    };

    let now = chrono::Utc::now().timestamp_millis();
    if let Some((val, updated_at)) = hit {
        if now - updated_at < DAY_MS {
            return Some(val);
        }
    }

    let key = {
        let conn = db.lock().unwrap();
        get_secret(&conn, "higgsfield").unwrap_or_default()
    };
    if key.is_empty() {
        return None;
    }

    if let Some(val) = hf::estimate(&key, &model.id, &body).await {
        let conn = db.lock().unwrap();
        let _ = write_cache(&conn, &cache_key, &val);
        Some(val)
    } else {
        None
    }
}

pub fn forget_estimates(db: &DbPool) {
    let conn = db.lock().unwrap();
    let _ = drop_cache(&conn, "estimate:");
}
