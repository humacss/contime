use crate::clock::monotonic_milliseconds;
use wasm_bindgen::prelude::*;
/// Thin clock and serialization boundary; shared Rust support owns the workload.
#[wasm_bindgen]
pub struct BudgetDemo {
    inner: crate::Session,
}
#[wasm_bindgen]
impl BudgetDemo {
    #[wasm_bindgen(constructor)]
    pub fn new(settings: &str, domain: &str) -> Result<Self, JsValue> {
        crate::Session::new(settings, domain).map(|inner| Self { inner }).map_err(|e| JsValue::from_str(&e))
    }
    pub fn input(&mut self, direction: &str) -> Result<(), JsValue> {
        self.inner.input(direction).map_err(|e| JsValue::from_str(&e))
    }
    pub fn frame(&mut self, elapsed_ms: f64, milliseconds: f64, max_steps: usize, stop: bool) -> Result<String, JsValue> {
        self.inner.frame(elapsed_ms, milliseconds, max_steps, stop, monotonic_milliseconds).map_err(|e| JsValue::from_str(&e))
    }
}
