//! Browser boundary; configuration stays fixed for a run.
use super::Config;
pub fn config(json: &str) -> Result<Config, String> {
    let object: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if !object.is_object() {
        return Err("Uptime settings must be an object".into());
    }
    let mut args = vec!["--execution".into(), "manual".into(), "--no-setup".into()];
    for name in
        ["services", "cadence_ms", "query_hz", "latency_ms", "jitter_ms", "min_delay_ms", "retention_ms", "checkpoint_interval", "duration"]
    {
        if let Some(value) = object.get(name) {
            if value.is_null() {
                continue;
            }
            let value = value.as_u64().ok_or_else(|| format!("{name} must be a nonnegative whole number"))?;
            args.extend([format!("--{}", name.replace('_', "-")), value.to_string()]);
        }
    }
    Config::parse(args)?.ok_or_else(|| "Missing uptime settings".into())
}
