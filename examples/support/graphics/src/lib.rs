mod animation;
pub mod bridge;
mod comparison;
pub mod dashboard;
mod dashboard_view;
mod entrance;
mod labels;
#[cfg(not(target_arch = "wasm32"))]
pub mod native;
mod navigation;
mod network;
mod palette;
mod paths;
mod playback;
#[cfg(not(target_arch = "wasm32"))]
mod process_memory;
pub mod protocol;
#[path = "../../movement/presentation.rs"]
pub mod report;
pub mod scene;
mod tracks;
mod transfer;
mod viewport;
#[cfg(target_arch = "wasm32")]
mod web;
