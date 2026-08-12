use wit_bindgen::FutureReader;
use crate::exports::astrobox::psys_plugin::{event_v3 as event, event_v3::EventType, lifecycle};

pub mod state;
pub mod ui;
pub mod barcode_image;

wit_bindgen::generate!({ path: "wit", world: "psys-world-v3", generate_all });
pub const QA_PKG: &str = "tw.einvoice.vehicle.single.band";
struct VehicleThreeSetter;

fn immediate_string(value: String) -> FutureReader<String> {
    let (writer, reader) = wit_future::new::<String>(String::new);
    wit_bindgen::spawn(async move { let _ = writer.write(value).await; });
    reader
}

fn immediate_unit() -> FutureReader<()> {
    let (writer, reader) = wit_future::new::<()>(|| ());
    wit_bindgen::spawn(async move { let _ = writer.write(()).await; });
    reader
}

fn reply(addr: &str, value: serde_json::Value) {
    let text = value.to_string();
    let _ = wit_bindgen::block_on(
        crate::astrobox::psys_host::interconnect::send_qaic_message(addr, QA_PKG, &text).into_future()
    );
}

fn handle_interconnect(payload: &str) {
    let outer: serde_json::Value = match serde_json::from_str(payload) {
        Ok(value) => value,
        Err(_) => return,
    };
    let inner_value = if let Some(text) = outer.get("payloadText").and_then(|v| v.as_str()) {
        serde_json::from_str(text).unwrap_or(serde_json::Value::Null)
    } else if let Some(value) = outer.get("payload") {
        if let Some(text) = value.as_str() {
            serde_json::from_str(text).unwrap_or(serde_json::Value::Null)
        } else {
            value.clone()
        }
    } else {
        outer.clone()
    };
    let envelope_addr = outer.get("addr").and_then(|v| v.as_str()).unwrap_or("");
    let addr = state::resolve_addr(envelope_addr);
    let tag = inner_value.get("tag").and_then(|v| v.as_str()).unwrap_or("");
    state::remember_message(&addr, tag);
    if addr.is_empty() {
        state::set_notice("已收到手環訊息，但 AstroBox 未提供裝置位址；請按重新偵測");
        ui::rerender();
        return;
    }
    match tag {
        "__hs__" => {
            let count = inner_value.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
            if count < 2 { reply(&addr, serde_json::json!({"tag":"__hs__", "count":count + 1})); }
        }
        "image-pull" => {
            state::mark_sync_pull_received();
            let request_id = inner_value.get("requestId").and_then(|v| v.as_u64()).unwrap_or(0);
            let card_index = inner_value.get("index")
                .or_else(|| inner_value.get("cardIndex"))
                .and_then(|v| v.as_u64()).unwrap_or(0).min(2) as usize;
            let ok = state::start_image_transfer(&addr, card_index, request_id);
            state::set_notice(if ok {
                format!("手環已請求{}，準備逐片傳送", state::card_name(card_index))
            } else {
                format!("無法啟動{}圖片傳輸", state::card_name(card_index))
            });
            ui::rerender();
        }
        "image-ready" => {
            let request_id = inner_value.get("requestId").and_then(|v| v.as_u64()).unwrap_or(0);
            let transfer_id = inner_value.get("transferId").and_then(|v| v.as_str()).unwrap_or("");
            let _ = state::continue_image_transfer(&addr, request_id, transfer_id, None);
        }
        "image-chunk-ack" => {
            let request_id = inner_value.get("requestId").and_then(|v| v.as_u64()).unwrap_or(0);
            let transfer_id = inner_value.get("transferId").and_then(|v| v.as_str()).unwrap_or("");
            let index = inner_value.get("index").and_then(|v| v.as_u64()).unwrap_or(u64::MAX) as usize;
            let _ = state::continue_image_transfer(&addr, request_id, transfer_id, Some(index));
        }
        "image-cancel" => state::cancel_image_transfer(&addr),
        "image-ack" => {
            let card_index = inner_value.get("index")
                .or_else(|| inner_value.get("cardIndex"))
                .and_then(|v| v.as_u64()).unwrap_or(0).min(2) as usize;
            state::set_notice(format!("手環已完整寫入並顯示{}圖片", state::card_name(card_index)));
            ui::rerender();
        }
        _ => {}
    }
}

impl event::Guest for VehicleThreeSetter {
    fn on_event(event_type: EventType, payload: String) -> FutureReader<String> {
        match event_type {
            EventType::InterconnectMessage => handle_interconnect(&payload),
            EventType::DeviceAction => { state::force_refresh_devices(); ui::rerender(); }
            EventType::Timer => {
                if state::handle_sync_timer(&payload) { ui::rerender(); }
            }
            _ => {}
        }
        immediate_string(String::new())
    }

    fn on_ui_event_v3(id: String, event: crate::astrobox::psys_host::ui_v3::Event, payload: String) -> FutureReader<String> {
        ui::handle_ui_event(&id, event, &payload);
        immediate_string(String::new())
    }

    fn on_ui_render(id: String) -> FutureReader<()> {
        ui::render_root(&id);
        let (writer, reader) = wit_future::new::<()>(|| ());
        wit_bindgen::spawn(async move {
            state::refresh_devices_async().await;
            ui::rerender();
            let _ = writer.write(()).await;
        });
        reader
    }

    fn on_card_render(_: String) -> FutureReader<()> { immediate_unit() }
}

impl lifecycle::Guest for VehicleThreeSetter {
    fn on_load() {
        let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).without_time().try_init();
        state::load();
        state::force_refresh_devices();
    }
}

export!(VehicleThreeSetter);
