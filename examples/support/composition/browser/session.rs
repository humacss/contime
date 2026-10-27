use crate::hash_example::setup::HashExample;
use crate::support::Settings;
use serde::Deserialize;
use std::time::Duration;

#[derive(Deserialize)]
struct Options {
    snapshots: usize,
    rate: usize,
    query_hz: usize,
    retention_ms: u64,
    checkpoint_interval: usize,
    #[serde(default)]
    max_pending: Option<serde_json::Value>,
    average_targets: String,
    max_targets: usize,
    #[serde(default)]
    snapshot_skew: usize,
}
fn settings(json: &str) -> Result<Settings, String> {
    let o: Options = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let mut args = Vec::new();
    for (key, value) in [
        ("--snapshots", o.snapshots.to_string()),
        ("--snapshot-skew", o.snapshot_skew.to_string()),
        ("--workers", "1".to_string()),
        ("--events-per-second", if o.rate == 0 { "max".into() } else { o.rate.to_string() }),
        ("--query-hz", o.query_hz.to_string()),
        ("--retention-ms", o.retention_ms.to_string()),
        ("--checkpoint-interval", o.checkpoint_interval.to_string()),
        ("--average-snapshots-per-event", o.average_targets),
        ("--max-snapshots-per-event", o.max_targets.to_string()),
    ] {
        args.extend([key.to_string(), value]);
    }
    if o.rate != 0 {
        if let Some(value) = o.max_pending {
            let pending: usize = serde_json::from_value(value).map_err(|e| e.to_string())?;
            args.extend(["--max-pending".to_string(), pending.to_string()]);
        }
    }
    Settings::parse(args, true).map_err(|e| e.to_string())?.map(|(s, _)| s).ok_or("missing settings".into())
}

enum Domain {
    Counter(Box<crate::frame::Frame>),
    Hash(Box<crate::frame::Frame<HashExample>>),
    Movement(Box<crate::movement_frame::Frame>),
}
impl crate::scheduler::Task for Domain {
    fn step(&mut self, now: Duration, stop: bool) -> Result<bool, String> {
        match self {
            Self::Counter(task) => task.step(now, stop),
            Self::Hash(task) => task.step(now, stop),
            Self::Movement(task) => task.step(now, stop),
        }
    }
    fn report(&self) -> serde_json::Value {
        match self {
            Self::Counter(task) => task.report(),
            Self::Hash(task) => task.report(),
            Self::Movement(task) => task.report(),
        }
    }
}
/// Select a case in support, then inject it into the browser's generic scheduler.
pub struct Session {
    inner: crate::scheduler::Session<Domain>,
}
impl Session {
    pub fn new(json: &str, domain: &str) -> Result<Self, String> {
        let task = if domain == "client-movement" {
            Domain::Movement(Box::new(crate::movement_frame::Frame::new(crate::movement::config(json)?)?))
        } else {
            let settings = settings(json)?;
            match domain {
                "counter" => Domain::Counter(Box::new(crate::frame::Frame::new(&settings)?)),
                "chain-hash" => Domain::Hash(Box::new(crate::frame::Frame::hash(&settings)?)),
                _ => return Err("Domain must be counter, chain-hash or client-movement".into()),
            }
        };
        Ok(Self { inner: crate::scheduler::Session::new(task) })
    }
    pub fn input(&mut self, direction: &str) -> Result<(), String> {
        match self.inner.task_mut() {
            Domain::Movement(task) => task.input(direction),
            _ => Err("Keyboard movement is only available for client-movement".into()),
        }
    }
    pub fn frame(
        &mut self,
        elapsed_ms: f64,
        milliseconds: f64,
        max_steps: usize,
        stop: bool,
        clock: impl FnMut() -> f64,
    ) -> Result<String, String> {
        self.inner.frame(elapsed_ms, milliseconds, max_steps, stop, clock)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn options(rate: usize) -> String {
        serde_json::json!({ "snapshots": 8, "rate": rate, "query_hz": 20,
            "retention_ms": 100, "checkpoint_interval": 4, "max_pending": 32,
            "average_targets": "1.1", "max_targets": 2 })
        .to_string()
    }
    fn report(session: &mut Session, elapsed: f64, stop: bool) -> serde_json::Value {
        serde_json::from_str(&session.frame(elapsed, 2.0, 16, stop, || 0.0).unwrap()).unwrap()
    }
    #[test]
    fn maximum_ignores_legacy_pending_capacity_and_accepts_its_absence() {
        let mut json: serde_json::Value = serde_json::from_str(&options(0)).unwrap();
        json.as_object_mut().unwrap().remove("max_pending");
        assert!(settings(&json.to_string()).is_ok());
        for value in [serde_json::json!(0), serde_json::json!(1), serde_json::json!("invalid")] {
            json["max_pending"] = value;
            assert!(settings(&json.to_string()).is_ok());
        }
    }
    #[test]
    fn numeric_rate_rejects_invalid_pending_capacity() {
        let mut json: serde_json::Value = serde_json::from_str(&options(100)).unwrap();
        for value in [serde_json::json!(0), serde_json::json!("invalid")] {
            json["max_pending"] = value;
            assert!(settings(&json.to_string()).is_err());
        }
    }
    #[test]
    fn host_budget_honors_clock_and_step_limits() {
        let mut session = Session::new(&options(0), "counter").unwrap();
        let r: serde_json::Value = serde_json::from_str(&session.frame(0.0, 2.0, 1, false, || 0.0).unwrap()).unwrap();
        assert_eq!(r["steps"], 1);
        let mut now = 0.0;
        let r: serde_json::Value = serde_json::from_str(
            &session
                .frame(1.0, 1.0, 100, false, || {
                    now += 0.25;
                    now
                })
                .unwrap(),
        )
        .unwrap();
        assert!(r["steps"].as_u64().unwrap() <= 3);
        assert!(session.frame(f64::NAN, 1.0, 1, false, || 0.0).is_err());
        assert!(session.frame(0.0, 0.0, 1, false, || 0.0).is_err());
    }
    #[test]
    fn continuous_domains_make_progress_then_stop_and_drain() {
        for domain in ["counter", "chain-hash"] {
            let mut session = Session::new(&options(1000), domain).unwrap();
            let mut last = serde_json::Value::Null;
            for frame in 0..200 {
                last = report(&mut session, frame as f64 * 16.0, false);
            }
            assert!(!last["done"].as_bool().unwrap());
            assert!(last["submitted"].as_u64().unwrap() > 0);
            assert!(last["completed"].as_u64().unwrap() > 0);
            let submitted = last["submitted"].as_u64().unwrap();
            for _ in 0..10000 {
                last = report(&mut session, 3200.0, true);
                if last["done"] == true {
                    break;
                }
            }
            assert_eq!(last["done"], true, "{domain}: {last}");
            assert_eq!(last["submitted"], submitted);
            assert_eq!(last["outstanding"], 0);
            assert_eq!(last["completed"], last["produced"]);
        }
    }
    #[test]
    fn every_common_browser_setting_reaches_the_shared_controller() {
        let json = serde_json::json!({ "snapshots": 8, "rate": 100, "query_hz": 17,
            "retention_ms": 321, "checkpoint_interval": 7, "max_pending": 64,
            "average_targets": "2.5", "max_targets": 4, "snapshot_skew": 90 })
        .to_string();
        let s = settings(&json).unwrap();
        assert_eq!(s.snapshot_count, 8);
        assert_eq!(s.workers, 1);
        assert_eq!(s.event_rate, crate::support::EventRate::PerSecond(100));
        assert_eq!(s.query_hz, 17);
        assert_eq!(s.retention_ms, 321);
        assert_eq!(s.checkpoint_interval, 7);
        assert_eq!(s.max_pending, 64);
        assert_eq!(s.average_targets.scaled(), 2500);
        assert_eq!(s.max_targets, 4);
        assert_eq!(s.snapshot_skew, 90);
        assert!(s.duration.is_none());
    }
    #[test]
    fn shared_settings_validate_before_start() {
        assert!(Session::new(&options(0), "unknown").is_err());
        assert!(Session::new("{}", "counter").is_err());
        let invalid = options(100).replace("\"max_pending\":32", "\"max_pending\":0");
        assert!(Session::new(&invalid, "counter").is_err());
    }
}

#[cfg(test)]
mod monitor_tests {
    use super::*;
    #[test]
    fn counter_uses_worker_feedback_and_cumulative_averages_before_any_query() {
        let json = serde_json::json!({"snapshots": 8, "rate": 1000, "query_hz": 1,
            "retention_ms": 20, "checkpoint_interval": 4, "max_pending": 64,
            "average_targets": "1.5", "max_targets": 3, "snapshot_skew": 90})
        .to_string();
        let mut session = Session::new(&json, "counter").unwrap();
        let mut last = serde_json::Value::Null;
        for frame in 0..100 {
            last = serde_json::from_str(&session.frame(frame as f64 * 16.0, 2.0, 64, false, || 0.0).unwrap()).unwrap();
        }
        assert!(last["completed"].as_u64().unwrap() > 0);
        assert_eq!(last["queries"], 0, "counter production must not depend on queries");
        let worker = &last["workers_status"][0];
        assert!(worker["total_inserted"].as_u64().unwrap() > 0);
        assert!(worker["total_replayed"].as_u64().unwrap() > 0);
        assert!(worker["avg_inserted_per_second"].as_f64().unwrap() > 0.0);
        assert!(worker["horizon"].as_u64().unwrap() > 0, "short retention must be driven");
        for _ in 0..1000 {
            last = serde_json::from_str(&session.frame(1600.0, 2.0, 1, true, || 0.0).unwrap()).unwrap();
            assert!(last["steps"].as_u64().unwrap() <= 1);
            if last["done"] == true {
                break;
            }
        }
        assert_eq!(last["done"], true);
        assert_eq!(last["produced"], last["completed"]);
        assert_eq!(last["outstanding"], 0);
        assert_eq!(last["queries"], 1);
        assert_eq!(last["completed"], last["produced"]);
    }
}

#[cfg(test)]
mod budget_monitor_tests {
    use super::*;
    #[test]
    fn exhausted_frame_budget_does_no_domain_work() {
        let json = serde_json::json!({"snapshots":8,"rate":0,"query_hz":1,"retention_ms":20,"checkpoint_interval":4,"average_targets":"1.0","max_targets":2}).to_string();
        for domain in ["counter", "chain-hash"] {
            let mut session = Session::new(&json, domain).unwrap();
            let mut clock = 0.0;
            let report: serde_json::Value = serde_json::from_str(
                &session
                    .frame(0.0, 0.01, 256, false, || {
                        clock += 0.02;
                        clock
                    })
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(report["steps"], 0);
            assert_eq!(report["produced"], 0);
            assert_eq!(report["queued"], 0);
            let report: serde_json::Value = serde_json::from_str(&session.frame(1.0, 1.0, 1, false, || 0.0).unwrap()).unwrap();
            assert_eq!(report["steps"], 1);
        }
    }
    #[test]
    fn common_counter_limits_remain_accepted() {
        let json = serde_json::json!({"snapshots":8,"rate":100,"query_hz":1,"retention_ms":86400000,"checkpoint_interval":1000000000,"max_pending":64,"average_targets":"1.0","max_targets":2}).to_string();
        assert!(Session::new(&json, "counter").is_ok());
    }
}

#[cfg(test)]
mod movement_tests {
    use super::*;
    #[test]
    fn movement_obeys_frame_steps_and_replays_delayed_keyboard_and_bot_inputs() {
        let json =
            serde_json::json!({"clients":4,"client_spacing":9,"movement_speed":10,"input_hz":30,"query_hz":20,"average_latency_ms":100,
            "jitter_ms":100,"trail_ms":200,"fade_ms":500,"retention_ms":1000,"checkpoint_interval":4})
            .to_string();
        let mut session = Session::new(&json, "client-movement").unwrap();
        session.input("east").unwrap();
        let mut initial = serde_json::Value::Null;
        for tick in 0..=5 {
            initial = serde_json::from_str(&session.frame(tick as f64 * 2.0, 1.0, 32, false, || 0.0).unwrap()).unwrap();
            if initial["movement"]["positions"].as_array().is_some_and(|p| p.len() == 4) {
                break;
            }
        }
        assert_eq!(initial["movement"]["positions"][1]["position"], serde_json::json!([9.0, 0.0]));
        assert_eq!(initial["movement"]["client_spacing"], 9);
        assert_eq!(initial["movement"]["movement_speed"], 10);
        let mut last = serde_json::Value::Null;
        for i in 1..180 {
            last = serde_json::from_str(&session.frame(i as f64 * 16.0, 1.0, 32, false, || 0.0).unwrap()).unwrap();
            assert!(last["steps"].as_u64().unwrap() <= 32);
        }
        assert!(last["movement"]["late"].as_u64().unwrap() > 0);
        assert!(last["movement"]["updates"].as_u64().unwrap() > 0);
        assert!(session.input("unknown").is_err());
        for i in 180..1000 {
            last = serde_json::from_str(&session.frame(i as f64 * 16.0, 1.0, 32, true, || 0.0).unwrap()).unwrap();
            if last["done"] == true {
                break;
            }
        }
        assert_eq!(last["done"], true, "{last}");
        assert_eq!(last["movement"]["keyboard_inputs"], 1);
        let x = last["movement"]["center"][0].as_f64().unwrap();
        assert!((28.0..30.0).contains(&x), "custom speed must travel ten units per second: {x}");
        assert_eq!(last["outstanding"], 0);
        assert_eq!(last["movement"]["positions"].as_array().unwrap().len(), 4);
    }
}
