use crate::{bridge::Bridge, report::Report};
use std::cell::RefCell;
use wasm_bindgen::prelude::*;
thread_local! { static BRIDGE: RefCell<Option<Bridge>> = const { RefCell::new(None) }; }
fn with_bridge<T>(f: impl FnOnce(&Bridge) -> T) -> Result<T, JsValue> {
    BRIDGE.with(|slot| slot.borrow().as_ref().map(f).ok_or_else(|| JsValue::from_str("Bevy presentation has not started")))
}
#[wasm_bindgen]
pub fn start_graphics(canvas: &str) -> Result<(), JsValue> {
    let bridge = Bridge::default();
    BRIDGE.with(|slot| {
        if slot.borrow().is_some() {
            return Err(JsValue::from_str("Bevy presentation already started"));
        }
        *slot.borrow_mut() = Some(bridge.clone());
        Ok(())
    })?;
    crate::scene::app(bridge, Some(canvas.into())).run();
    Ok(())
}
#[wasm_bindgen]
pub fn update_scene(json: &str, reduced_motion: bool) -> Result<(), JsValue> {
    let report: Report = serde_json::from_str(json).map_err(|e| JsValue::from_str(&e.to_string()))?;
    with_bridge(|bridge| {
        bridge.reduced_motion(reduced_motion);
        bridge.publish(report)
    })?
    .map_err(|e| JsValue::from_str(&e))
}
#[wasm_bindgen]
pub fn set_running(running: bool) -> Result<(), JsValue> {
    with_bridge(|bridge| bridge.running(running))
}
#[wasm_bindgen]
pub fn reset_scene() -> Result<(), JsValue> {
    with_bridge(Bridge::restart_view)
}

#[wasm_bindgen]
pub fn finish_scene() -> Result<(), JsValue> {
    with_bridge(Bridge::finish)
}

#[wasm_bindgen]
pub fn update_uptime(json: &str, reduced_motion: bool) -> Result<(), JsValue> {
    let report: crate::uptime_report::Report = serde_json::from_str(json).map_err(|e| JsValue::from_str(&e.to_string()))?;
    with_bridge(|bridge| {
        bridge.reduced_motion(reduced_motion);
        bridge.publish_uptime(report)
    })?
    .map_err(|e| JsValue::from_str(&e))
}

#[wasm_bindgen]
pub fn show_visualizations() -> Result<(), JsValue> {
    with_bridge(Bridge::show_chooser)
}
#[wasm_bindgen]
pub fn take_visualization() -> Result<Option<String>, JsValue> {
    with_bridge(|bridge| bridge.take_selection().map(|choice| choice.slug().to_owned()))
}

#[wasm_bindgen]
pub fn take_navigation() -> Result<Option<String>, JsValue> {
    with_bridge(|bridge| bridge.take_navigation().map(|navigation| navigation.slug().to_owned()))
}

#[wasm_bindgen]
pub fn update_accounts(json: &str, reduced_motion: bool) -> Result<(), JsValue> {
    let report: crate::accounts_report::Report = serde_json::from_str(json).map_err(|e| JsValue::from_str(&e.to_string()))?;
    with_bridge(|bridge| {
        bridge.reduced_motion(reduced_motion);
        bridge.publish_accounts(report)
    })?
    .map_err(|e| JsValue::from_str(&e))
}

#[wasm_bindgen]
pub fn update_robotics(json: &str, reduced_motion: bool) -> Result<(), JsValue> {
    let report: crate::robotics_report::Report = serde_json::from_str(json).map_err(|e| JsValue::from_str(&e.to_string()))?;
    with_bridge(|bridge| {
        bridge.reduced_motion(reduced_motion);
        bridge.publish_robotics(report)
    })?
    .map_err(|e| JsValue::from_str(&e))
}
