mod animation;
pub mod bridge;
#[path = "../../visualizer_catalog.rs"]
pub mod catalog;
mod chooser;
mod comparison;
mod entrance;
mod explanation;
mod labels;
#[cfg(not(target_arch = "wasm32"))]
pub mod native;
mod navigation;
mod network;
mod palette;
mod paths;
mod playback;
pub mod protocol;
#[path = "../../movement/presentation.rs"]
pub mod report;
pub mod scene;
mod tracks;
mod transfer;
#[cfg(target_arch = "wasm32")]
mod web;

mod uptime;
#[path = "../../uptime/report.rs"]
pub mod uptime_report;

mod timeline;

mod viewport;

mod accounts;
#[path = "../../accounts/report.rs"]
pub mod accounts_report;

mod robotics;
#[path = "../../robotics/report.rs"]
pub mod robotics_report;
