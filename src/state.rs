use std::collections::HashMap;
use std::fs;
use std::sync::{Mutex, OnceLock};
use serde::{Deserialize, Serialize};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use crate::astrobox::psys_host::{device, interconnect, register, thirdpartyapp, timer};

// Keep the successful single-card config path so upgrading preserves the
// existing invoice carrier. New fields deserialize to empty strings.
const CONFIG_PATH: &str = "./einvoice-vehicle-single.config.json";
const RAW_CHUNK: usize = 3 * 1024;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(rename = "vehicleBarcode", default)]
    pub vehicle_barcode: String,
    #[serde(rename = "openpointBarcode", default)]
    pub openpoint_barcode: String,
    #[serde(rename = "familyBarcode", default)]
    pub family_barcode: String,
    #[serde(rename = "updatedAt", default)]
    pub updated_at: u64,
}

pub struct ImageTransfer {
    pub card_index: usize,
    pub request_id: u64,
    pub transfer_id: String,
    pub chunks: Vec<String>,
    pub next_index: usize,
}

pub struct AppState {
    pub config: Config,
    pub devices: Vec<(String, String)>,
    pub paired_devices: Vec<(String, String)>,
    pub connected_count: usize,
    pub registration_ok: usize,
    pub registration_total: usize,
    pub app_found: usize,
    pub app_query_ok: usize,
    pub receive_count: u64,
    pub last_receive_tag: String,
    pub recent_addrs: Vec<String>,
    pub image_transfers: HashMap<String, ImageTransfer>,
    pub sync_generation: u64,
    pub sync_attempt: u8,
    pub sync_waiting_for_pull: bool,
    pub root_element: Option<String>,
    pub notice: Option<String>,
    pub vehicle_input: String,
    pub openpoint_input: String,
    pub family_input: String,
}

static STATE: OnceLock<Mutex<AppState>> = OnceLock::new();
pub fn state() -> &'static Mutex<AppState> {
    STATE.get_or_init(|| Mutex::new(AppState {
        config: Config::default(),
        devices: vec![],
        paired_devices: vec![],
        connected_count: 0,
        registration_ok: 0,
        registration_total: 0,
        app_found: 0,
        app_query_ok: 0,
        receive_count: 0,
        last_receive_tag: String::new(),
        recent_addrs: vec![],
        image_transfers: HashMap::new(),
        sync_generation: 0,
        sync_attempt: 0,
        sync_waiting_for_pull: false,
        root_element: None,
        notice: None,
        vehicle_input: String::new(),
        openpoint_input: String::new(),
        family_input: String::new(),
    }))
}

pub fn card_name(index: usize) -> &'static str {
    match index {
        1 => "7-ELEVEN OPENPOINT",
        2 => "全家會員",
        _ => "發票載具",
    }
}

pub fn load() {
    let config = fs::read_to_string(CONFIG_PATH).ok()
        .and_then(|s| serde_json::from_str::<Config>(&s).ok())
        .unwrap_or_default();
    let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
    st.vehicle_input = config.vehicle_barcode.clone();
    st.openpoint_input = config.openpoint_barcode.clone();
    st.family_input = config.family_barcode.clone();
    st.config = config;
}

pub fn save() -> Result<(), String> {
    let config = state().lock().unwrap_or_else(|p| p.into_inner()).config.clone();
    let text = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    fs::write(CONFIG_PATH, text).map_err(|e| e.to_string())
}

pub fn set_notice(text: impl Into<String>) {
    state().lock().unwrap_or_else(|p| p.into_inner()).notice = Some(text.into());
}

pub async fn refresh_devices_async() {
    let connected = device::get_connected_device_list().into_future().await;
    let connected_count = connected.len();
    let paired = if connected_count == 0 {
        device::get_device_list().into_future().await
    } else {
        vec![]
    };
    {
        let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
        st.connected_count = connected_count;
        st.devices = connected.into_iter().map(|d| (d.addr, d.name)).collect();
        st.paired_devices = paired.into_iter().map(|d| (d.addr, d.name)).collect();
    }
    refresh_installed_apps_async().await;
    ensure_registered_async().await;
}

pub async fn refresh_installed_apps_async() -> (usize, usize) {
    let addrs = state().lock().unwrap_or_else(|p| p.into_inner())
        .devices.iter().map(|d| d.0.clone()).collect::<Vec<_>>();
    let mut query_ok = 0usize;
    let mut found = 0usize;
    for addr in addrs {
        if let Ok(apps) = thirdpartyapp::get_thirdparty_app_list(&addr).into_future().await {
            query_ok += 1;
            if apps.iter().any(|app| app.package_name == crate::QA_PKG) { found += 1; }
        }
    }
    let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
    st.app_query_ok = query_ok;
    st.app_found = found;
    (found, query_ok)
}

pub fn force_refresh_installed_apps() -> (usize, usize) {
    let addrs = state().lock().unwrap_or_else(|p| p.into_inner())
        .devices.iter().map(|d| d.0.clone()).collect::<Vec<_>>();
    let mut query_ok = 0usize;
    let mut found = 0usize;
    for addr in addrs {
        if let Ok(apps) = wit_bindgen::block_on(
            thirdpartyapp::get_thirdparty_app_list(&addr).into_future()
        ) {
            query_ok += 1;
            if apps.iter().any(|app| app.package_name == crate::QA_PKG) { found += 1; }
        }
    }
    let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
    st.app_query_ok = query_ok;
    st.app_found = found;
    (found, query_ok)
}

pub async fn ensure_registered_async() -> (usize, usize) {
    let addrs = state().lock().unwrap_or_else(|p| p.into_inner())
        .devices.iter().map(|d| d.0.clone()).collect::<Vec<_>>();
    let total = addrs.len();
    let mut ok = 0usize;
    for addr in addrs {
        if register::register_interconnect_recv(&addr, crate::QA_PKG)
            .into_future().await.is_ok() { ok += 1; }
    }
    let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
    st.registration_ok = ok;
    st.registration_total = total;
    (ok, total)
}

pub fn force_reregister() -> (usize, usize) {
    let addrs = state().lock().unwrap_or_else(|p| p.into_inner())
        .devices.iter().map(|d| d.0.clone()).collect::<Vec<_>>();
    let total = addrs.len();
    let mut ok = 0usize;
    for addr in addrs {
        if wit_bindgen::block_on(
            register::register_interconnect_recv(&addr, crate::QA_PKG).into_future()
        ).is_ok() { ok += 1; }
    }
    let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
    st.registration_ok = ok;
    st.registration_total = total;
    (ok, total)
}

pub fn force_refresh_devices() {
    let connected = wit_bindgen::block_on(device::get_connected_device_list().into_future());
    let connected_count = connected.len();
    let paired = if connected_count == 0 {
        wit_bindgen::block_on(device::get_device_list().into_future())
    } else {
        vec![]
    };
    {
        let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
        st.connected_count = connected_count;
        st.devices = connected.into_iter().map(|d| (d.addr, d.name)).collect();
        st.paired_devices = paired.into_iter().map(|d| (d.addr, d.name)).collect();
    }
    force_refresh_installed_apps();
    force_reregister();
}

pub fn remember_message(addr: &str, tag: &str) {
    let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
    st.receive_count = st.receive_count.saturating_add(1);
    st.last_receive_tag = tag.to_string();
    if !addr.is_empty() && !st.recent_addrs.iter().any(|x| x == addr) {
        st.recent_addrs.push(addr.to_string());
    }
}

pub fn resolve_addr(envelope_addr: &str) -> String {
    if !envelope_addr.is_empty() { return envelope_addr.to_string(); }
    let st = state().lock().unwrap_or_else(|p| p.into_inner());
    if st.devices.len() == 1 { return st.devices[0].0.clone(); }
    if let Some(addr) = st.recent_addrs.last() { return addr.clone(); }
    String::new()
}

fn send_json(addr: &str, value: serde_json::Value) -> bool {
    let text = value.to_string();
    wit_bindgen::block_on(interconnect::send_qaic_message(addr, crate::QA_PKG, &text).into_future()).is_ok()
}

fn known_addrs(explicit: &str) -> Vec<String> {
    if !explicit.is_empty() { return vec![explicit.to_string()]; }
    let st = state().lock().unwrap_or_else(|p| p.into_inner());
    let mut out = st.devices.iter().map(|d| d.0.clone()).collect::<Vec<_>>();
    for addr in &st.recent_addrs {
        if !out.iter().any(|x| x == addr) { out.push(addr.clone()); }
    }
    out
}

fn send_refresh_once() -> usize {
    let mut sent = 0usize;
    for target in known_addrs("") {
        if send_json(&target, serde_json::json!({"tag":"image-refresh", "all":true})) { sent += 1; }
    }
    sent
}

fn launch_band_app() -> usize {
    let addrs = state().lock().unwrap_or_else(|p| p.into_inner())
        .devices.iter().map(|d| d.0.clone()).collect::<Vec<_>>();
    let mut launched = 0usize;
    for addr in addrs {
        let apps = wit_bindgen::block_on(thirdpartyapp::get_thirdparty_app_list(&addr).into_future());
        let Ok(apps) = apps else { continue; };
        let Some(app) = apps.into_iter().find(|app| app.package_name == crate::QA_PKG) else { continue; };
        if wit_bindgen::block_on(thirdpartyapp::launch_qa(&addr, &app, "pages/index").into_future()).is_ok() {
            launched += 1;
        }
    }
    launched
}

fn arm_sync_retry(generation: u64, attempt: u8) {
    let payload = serde_json::json!({
        "action":"image-refresh-retry", "generation":generation, "attempt":attempt
    }).to_string();
    let _ = wit_bindgen::block_on(timer::set_timeout(1200, &payload).into_future());
}

pub fn begin_reliable_sync() -> (usize, usize) {
    force_refresh_devices();
    let launched = launch_band_app();
    let generation = {
        let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
        st.sync_generation = st.sync_generation.saturating_add(1);
        st.sync_attempt = 1;
        st.sync_waiting_for_pull = true;
        st.sync_generation
    };
    let sent = send_refresh_once();
    arm_sync_retry(generation, 2);
    (sent, launched)
}

pub fn handle_sync_timer(payload: &str) -> bool {
    let outer = serde_json::from_str::<serde_json::Value>(payload).unwrap_or(serde_json::Value::Null);
    let raw = outer.get("payload").and_then(|v| v.as_str()).unwrap_or(payload);
    let value = serde_json::from_str::<serde_json::Value>(raw).unwrap_or(serde_json::Value::Null);
    if value.get("action").and_then(|v| v.as_str()) != Some("image-refresh-retry") { return false; }
    let generation = value.get("generation").and_then(|v| v.as_u64()).unwrap_or(0);
    let attempt = value.get("attempt").and_then(|v| v.as_u64()).unwrap_or(0).min(255) as u8;
    {
        let st = state().lock().unwrap_or_else(|p| p.into_inner());
        if !st.sync_waiting_for_pull || st.sync_generation != generation { return true; }
    }
    let sent = send_refresh_once();
    {
        let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
        if !st.sync_waiting_for_pull || st.sync_generation != generation { return true; }
        st.sync_attempt = attempt;
    }
    if attempt < 5 {
        arm_sync_retry(generation, attempt + 1);
        set_notice(format!("等待手環回應：已送出第 {attempt}/5 次同步通知（成功 {sent}）"));
    } else {
        let notice = if sent > 0 {
            String::from("已送出 5 次同步通知但手環尚未回拉；請確認手環已停留在「隨身條碼」，再按重新推送")
        } else {
            String::from("同步通知發送失敗；請重新連接 AstroBox 後再試")
        };
        set_notice(notice);
    }
    true
}

pub fn mark_sync_pull_received() {
    let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
    st.sync_waiting_for_pull = false;
}

fn send_transfer_chunk(addr: &str) -> bool {
    let packet = {
        let st = state().lock().unwrap_or_else(|p| p.into_inner());
        let Some(t) = st.image_transfers.get(addr) else { return false; };
        let Some(chunk) = t.chunks.get(t.next_index) else { return false; };
        serde_json::json!({
            "tag":"image-data", "type":"data", "cardIndex":t.card_index,
            "requestId":t.request_id, "transferId":t.transfer_id,
            "index":t.next_index, "chunk":chunk
        })
    };
    send_json(addr, packet)
}

pub fn start_image_transfer(addr: &str, card_index: usize, request_id: u64) -> bool {
    let config = state().lock().unwrap_or_else(|p| p.into_inner()).config.clone();
    let value = match card_index {
        1 => config.openpoint_barcode.clone(),
        2 => config.family_barcode.clone(),
        _ => config.vehicle_barcode.clone(),
    };
    if value.is_empty() {
        return send_json(addr, serde_json::json!({
            "tag":"image-error", "cardIndex":card_index, "requestId":request_id,
            "message":format!("{}尚未在手機設定", card_name(card_index))
        }));
    }
    let png = match crate::barcode_image::render_png(&value, card_index) {
        Ok(bytes) => bytes,
        Err(_) => return send_json(addr, serde_json::json!({
            "tag":"image-error", "cardIndex":card_index, "requestId":request_id,
            "message":format!("{}圖片產生失敗", card_name(card_index))
        })),
    };
    let chunks = png.chunks(RAW_CHUNK).map(|x| STANDARD.encode(x)).collect::<Vec<_>>();
    let transfer_id = format!("three-v140-{request_id}-{card_index}-{}-{}", config.updated_at, png.len());
    let total_chunks = chunks.len();
    {
        let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
        st.image_transfers.insert(addr.to_string(), ImageTransfer {
            card_index,
            request_id,
            transfer_id: transfer_id.clone(),
            chunks,
            next_index: 0,
        });
    }
    send_json(addr, serde_json::json!({
        "tag":"image-header", "type":"header", "cardIndex":card_index,
        "requestId":request_id, "transferId":transfer_id,
        "totalSize":png.len(), "chunkSize":RAW_CHUNK, "totalChunks":total_chunks,
        "width":190, "height":480, "mime":"image/png", "rotation":180
    }))
}

pub fn continue_image_transfer(addr: &str, request_id: u64, transfer_id: &str, ack_index: Option<usize>) -> bool {
    let finished = {
        let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
        let Some(t) = st.image_transfers.get_mut(addr) else { return false; };
        if t.request_id != request_id || t.transfer_id != transfer_id { return false; }
        if let Some(index) = ack_index {
            if index != t.next_index { return false; }
            t.next_index += 1;
        } else if t.next_index != 0 {
            return false;
        }
        t.next_index >= t.chunks.len()
    };
    if !finished { return send_transfer_chunk(addr); }
    let end = {
        let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
        let Some(t) = st.image_transfers.remove(addr) else { return false; };
        serde_json::json!({
            "tag":"image-end", "type":"end", "cardIndex":t.card_index,
            "requestId":t.request_id, "transferId":t.transfer_id,
            "totalChunks":t.chunks.len()
        })
    };
    send_json(addr, end)
}

pub fn cancel_image_transfer(addr: &str) {
    state().lock().unwrap_or_else(|p| p.into_inner()).image_transfers.remove(addr);
}
