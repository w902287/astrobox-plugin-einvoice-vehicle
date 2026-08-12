use crate::astrobox::psys_host::ui_v3 as ui;
use crate::state;
use std::time::{SystemTime, UNIX_EPOCH};

const EV_VEHICLE: &str = "vehicle_input";
const EV_OPENPOINT: &str = "openpoint_input";
const EV_FAMILY: &str = "family_input";
const EV_SAVE: &str = "save_push";
const EV_PUSH: &str = "push_again";
const EV_REFRESH: &str = "refresh_device";
const BG: &str = "#07100c";
const CARD: &str = "#142019";
const GREEN: &str = "#63e68b";
const TEXT: &str = "#f4fff7";
const MUTED: &str = "#9aad9f";
const RED: &str = "#ff8b86";

pub fn render_root(id: &str) {
    state::state().lock().unwrap_or_else(|p| p.into_inner()).root_element = Some(id.to_string());
    ui::render(id, build_ui());
}

pub fn rerender() {
    let root = state::state().lock().unwrap_or_else(|p| p.into_inner()).root_element.clone();
    if let Some(id) = root { ui::render(&id, build_ui()); }
}

fn input_value(payload: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) {
        for key in ["value", "content", "text"] {
            if let Some(text) = value.get(key).and_then(|x| x.as_str()) { return text.into(); }
        }
        if let Some(text) = value.as_str() { return text.into(); }
    }
    payload.into()
}

fn compact(raw: &str) -> String {
    raw.trim().chars().filter(|c| !c.is_whitespace()).collect::<String>()
}

fn normalize_vehicle(raw: &str) -> Result<String, String> {
    let mut value = compact(raw).to_ascii_uppercase();
    if value.is_empty() { return Ok(String::new()); }
    if !value.starts_with('/') { value.insert(0, '/'); }
    if value.len() != 8 { return Err("發票載具格式應為 / 加 7 碼，共 8 個字元".into()); }
    if !value[1..].chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+')) {
        return Err("發票載具只能包含英數字、.、-、+".into());
    }
    Ok(value)
}

fn normalize_openpoint(raw: &str) -> Result<String, String> {
    // OPENPOINT/uniopen most commonly uses GID + 14 digits, but legacy and
    // linked physical membership cards can expose other Code 128 values.
    // Preserve the exact case and validate only what our Code 128 renderer
    // can encode reliably.
    let value = compact(raw);
    if value.is_empty() { return Ok(String::new()); }
    if !(4..=24).contains(&value.len()) || !value.chars().all(|c| c.is_ascii_graphic()) {
        return Err("OPENPOINT 請照 App 會員條碼下方完整輸入（不限 GID，4–24 位英數或符號）".into());
    }
    Ok(value)
}

fn normalize_family(raw: &str) -> Result<String, String> {
    let value = compact(raw);
    if value.is_empty() { return Ok(String::new()); }
    if value.len() == 10 && value.starts_with("09") && value.chars().all(|c| c.is_ascii_digit()) {
        return Ok(value);
    }
    if !(8..=20).contains(&value.len()) || !value.chars().all(|c| c.is_ascii_graphic()) {
        return Err("全家會員請填 09 開頭 10 位手機號，或 App 顯示的 8–20 位會員碼".into());
    }
    Ok(value)
}

pub fn handle_ui_event(id: &str, event: ui::Event, payload: &str) {
    match event {
        ui::Event::Input | ui::Event::Change => {
            let value = input_value(payload);
            let mut st = state::state().lock().unwrap_or_else(|p| p.into_inner());
            match id {
                EV_VEHICLE => st.vehicle_input = value,
                EV_OPENPOINT => st.openpoint_input = value,
                EV_FAMILY => st.family_input = value,
                _ => {}
            }
        }
        ui::Event::Click => match id {
            EV_SAVE => save_and_sync(),
            EV_PUSH => push_again(),
            EV_REFRESH => {
                state::force_refresh_devices();
                let (connected, app_found, query_ok, registered, total) = {
                    let st = state::state().lock().unwrap_or_else(|p| p.into_inner());
                    (st.connected_count, st.app_found, st.app_query_ok, st.registration_ok, st.registration_total)
                };
                state::set_notice(format!(
                    "偵測：連線 {connected}、RPK {app_found}（查詢成功 {query_ok}）、路由 {registered}/{total}"
                ));
                rerender();
            }
            _ => {}
        },
        _ => {}
    }
}

fn save_and_sync() {
    let (raw_vehicle, raw_openpoint, raw_family) = {
        let st = state::state().lock().unwrap_or_else(|p| p.into_inner());
        (st.vehicle_input.clone(), st.openpoint_input.clone(), st.family_input.clone())
    };
    let vehicle = match normalize_vehicle(&raw_vehicle) {
        Ok(value) => value,
        Err(error) => { state::set_notice(error); rerender(); return; }
    };
    let openpoint = match normalize_openpoint(&raw_openpoint) {
        Ok(value) => value,
        Err(error) => { state::set_notice(error); rerender(); return; }
    };
    let family = match normalize_family(&raw_family) {
        Ok(value) => value,
        Err(error) => { state::set_notice(error); rerender(); return; }
    };
    if vehicle.is_empty() && openpoint.is_empty() && family.is_empty() {
        state::set_notice("請至少填寫一組條碼"); rerender(); return;
    }
    let updated_at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    {
        let mut st = state::state().lock().unwrap_or_else(|p| p.into_inner());
        st.vehicle_input = vehicle.clone();
        st.openpoint_input = openpoint.clone();
        st.family_input = family.clone();
        st.config.vehicle_barcode = vehicle;
        st.config.openpoint_barcode = openpoint;
        st.config.family_barcode = family;
        st.config.updated_at = updated_at;
    }
    match state::save() {
        Ok(_) => {
            let (count, launched) = state::begin_reliable_sync();
            state::set_notice(if count > 0 {
                format!("設定已儲存；已啟動 {launched} 台手環 App，送出第 1/5 次同步通知（成功 {count}）")
            } else {
                "設定已儲存，但同步通知發送失敗；請重新連接 AstroBox 後再試".into()
            });
        }
        Err(error) => state::set_notice(format!("儲存失敗：{error}")),
    }
    rerender();
}

fn push_again() {
    let has_any = {
        let st = state::state().lock().unwrap_or_else(|p| p.into_inner());
        !st.config.vehicle_barcode.is_empty()
            || !st.config.openpoint_barcode.is_empty()
            || !st.config.family_barcode.is_empty()
    };
    if !has_any {
        state::set_notice("請先輸入並儲存至少一組條碼");
    } else {
        let (count, launched) = state::begin_reliable_sync();
        state::set_notice(if count > 0 {
            format!("已啟動 {launched} 台手環 App，送出第 1/5 次同步通知（成功 {count}）")
        } else {
            "同步通知發送失敗；請重新連接 AstroBox 後再試".into()
        });
    }
    rerender();
}

fn field(label: &str, hint: &str, value: &str, id: &str) -> ui::Element {
    ui::Element::new(ui::ElementType::Div, None).bg(CARD).padding(10).radius(11).margin_top(8)
        .child(ui::Element::new(ui::ElementType::P, Some(label)).size(15).text_color(TEXT))
        .child(ui::Element::new(ui::ElementType::P, Some(hint)).size(11).text_color(MUTED))
        .child(ui::Element::new(ui::ElementType::Input, Some(value))
            .prop("placeholder", hint).width_full()
            .on(ui::Event::Input, id).on(ui::Event::Change, id))
}

fn build_ui() -> ui::Element {
    let st = state::state().lock().unwrap_or_else(|p| p.into_inner());
    let device_text = if st.devices.is_empty() {
        if st.paired_devices.is_empty() {
            "未偵測到手環".into()
        } else {
            format!("僅找到已配對：{}（目前非實際連線）",
                st.paired_devices.iter().map(|x| x.1.as_str()).collect::<Vec<_>>().join("、"))
        }
    } else {
        format!("已連線：{} · RPK {}/{} · 路由 {}/{}",
            st.devices.iter().map(|x| x.1.as_str()).collect::<Vec<_>>().join("、"),
            st.app_found, st.connected_count, st.registration_ok, st.registration_total)
    };
    let healthy = st.connected_count > 0
        && st.app_found == st.connected_count
        && st.registration_ok == st.registration_total;
    let device = ui::Element::new(ui::ElementType::Div, None)
        .flex().flex_direction(ui::FlexDirection::Row).gap(8).align_center().margin_top(8)
        .child(ui::Element::new(ui::ElementType::P, Some(&device_text)).size(13)
            .text_color(if healthy { GREEN } else { RED }))
        .child(ui::Element::new(ui::ElementType::Button, Some("重新偵測")).bg(CARD).text_color(TEXT)
            .radius(10).on(ui::Event::Click, EV_REFRESH));
    let actions = ui::Element::new(ui::ElementType::Div, None)
        .flex().flex_direction(ui::FlexDirection::Row).gap(8).margin_top(12)
        .child(ui::Element::new(ui::ElementType::Button, Some("儲存三組並同步")).bg(GREEN)
            .text_color("#061009").radius(11).on(ui::Event::Click, EV_SAVE))
        .child(ui::Element::new(ui::ElementType::Button, Some("重新推送")).bg(CARD)
            .text_color(TEXT).radius(11).on(ui::Event::Click, EV_PUSH));
    let notice = st.notice.clone().unwrap_or_else(|| {
        if st.receive_count == 0 {
            "左右滑動切換三張；空白欄位會保留該卡舊圖片。".into()
        } else {
            format!("已收到 {} 個手環訊息；最後標籤：{}", st.receive_count, st.last_receive_tag)
        }
    });
    ui::Element::new(ui::ElementType::Div, None)
        .flex().flex_direction(ui::FlexDirection::Column).width_full().bg(BG).padding(14).radius(12)
        .child(ui::Element::new(ui::ElementType::P, Some("隨身條碼設定 v1.4.3 · 可靠覆蓋同步")).size(23).text_color(GREEN))
        .child(ui::Element::new(ui::ElementType::P, Some("發票載具 · OPENPOINT · 全家會員")).size(13).text_color(MUTED))
        .child(device)
        .child(field("① 發票手機條碼載具", "/ 加 7 碼，例如 /ABC1234", &st.vehicle_input, EV_VEHICLE))
        .child(field("② 7-ELEVEN OPENPOINT", "照 App 會員條碼下方完整輸入（不限 GID）", &st.openpoint_input, EV_OPENPOINT))
        .child(field("③ 全家會員", "09 開頭 10 位手機號，或 App 會員碼", &st.family_input, EV_FAMILY))
        .child(actions)
        .child(ui::Element::new(ui::ElementType::P, Some(&notice)).size(13).text_color(GREEN).margin_top(9))
}
