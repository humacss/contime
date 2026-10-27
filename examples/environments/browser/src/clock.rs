//! Browser monotonic clock; application composition supplies the work.
use wasm_bindgen::prelude::*;
#[wasm_bindgen(inline_js = "export function monotonic_milliseconds() { return performance.now(); }")]
extern "C" {
    pub fn monotonic_milliseconds() -> f64;
}
