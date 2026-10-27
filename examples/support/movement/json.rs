//! Browser boundary: translate case settings into the shared command-line parser.
use super::Config;
pub fn config(json: &str) -> Result<Config, String> {
    let object: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if !object.is_object() {
        return Err("Movement settings must be an object".into());
    }
    let mut args = vec!["--execution".into(), "manual".into(), "--no-setup".into()];
    for name in [
        "clients",
        "client_spacing",
        "movement_speed",
        "input_hz",
        "query_hz",
        "average_latency_ms",
        "jitter_ms",
        "trail_ms",
        "fade_ms",
        "retention_ms",
        "checkpoint_interval",
    ] {
        if let Some(value) = object.get(name) {
            let value = value.as_u64().ok_or_else(|| format!("{name} must be a nonnegative whole number"))?;
            args.extend([format!("--{}", name.replace('_', "-")), value.to_string()]);
        }
    }
    Config::parse(args)?.ok_or_else(|| "Missing movement settings".into())
}
