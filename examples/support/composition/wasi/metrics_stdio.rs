//! Exercise the guest command's actual stdout contract, including validated fast exit.
use contime_wasm_example::support::monitored_wire::Sample;
use std::process::Command;

fn check_stream(execution: &str, domain: &str) {
    let workers = if execution == "runtime" { 2 } else { 1 };
    let workers_arg = workers.to_string();
    let output = Command::new(env!("CARGO_BIN_EXE_contime-wasm-example"))
        .args([
            "--metrics-stdio",
            "--domain",
            domain,
            "--execution",
            execution,
            "--workers",
            &workers_arg,
            "--snapshot-skew",
            "100",
            "--snapshots",
            "16",
            "--events-per-second",
            "100",
            "--query-hz",
            "1",
            "--duration",
            "1",
        ])
        .output()
        .expect("run guest");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let samples: Vec<_> = stdout.lines().map(|line| Sample::decode(line).expect("stdout contains only samples")).collect();
    assert!(samples.len() >= 10, "expected frequent reports: {}", samples.len());
    assert!(!samples[0].done);
    assert_eq!(samples[0].metrics.produced, 0);
    assert!(samples[..samples.len() - 1].iter().all(|sample| !sample.done));
    let last = samples.last().unwrap();
    assert!(last.done);
    assert!(last.metrics.produced > 0);
    assert_eq!(last.metrics.outstanding, 0);
    assert_eq!(last.metrics.completed, last.metrics.produced);
    assert_eq!(last.snapshot_count, 16);
    assert_eq!(last.snapshots.len(), 16);
    for snapshot in &last.snapshots {
        assert_eq!(snapshot.produced_count, snapshot.verified_count);
        assert_eq!(snapshot.produced_value, snapshot.verified_value);
        assert_eq!(snapshot.produced_hash, snapshot.verified_hash);
        assert_eq!(snapshot.produced_hash.is_some(), domain == "chain-hash");
    }
    if domain == "chain-hash" {
        assert!(last.metrics.reversed.unwrap() > 0);
        assert!(last.metrics.queries > 1);
        assert!(last.snapshots.iter().any(|snapshot| snapshot.verified_hash.is_some_and(|hash| hash != [0; 32])));
    }
    let reading = last.reading.as_ref().unwrap();
    assert_eq!(reading.workers.len(), workers);
    assert_eq!(reading.workers.iter().map(|w| w.history_stores).sum::<usize>(), 1);
    assert!(reading.workers.iter().all(|w| w.pending_histories == 0));
    assert_eq!(reading.workers.iter().map(|w| w.counters.unwrap().inserted_applications).sum::<u64>(), last.metrics.produced);
    assert!(reading.workers.iter().map(|w| w.counters.unwrap().replay_applications.unwrap()).sum::<u64>() >= last.metrics.produced);
}

#[test]
fn manual_guest_streams_metrics_until_drained() {
    check_stream("manual", "counter");
}

#[cfg(feature = "runtime")]
#[test]
fn runtime_guest_streams_metrics_until_validated_exit() {
    check_stream("runtime", "counter");
}

#[test]
fn hash_guest_uses_monitored_transport() {
    check_stream("manual", "chain-hash");
}

#[cfg(feature = "runtime")]
#[test]
fn runtime_hash_guest_uses_monitored_transport() {
    check_stream("runtime", "chain-hash");
}

#[test]
fn hash_text_output_preserves_full_verified_digests() {
    let output = Command::new(env!("CARGO_BIN_EXE_contime-wasm-example"))
        .args(["--domain", "chain-hash", "--snapshots", "8", "--events-per-second", "100", "--duration", "1"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("DRAINED (validated)"));
    let final_report = stdout.rsplit("DRAINED (validated)").next().unwrap();
    let hashes: Vec<_> = final_report.lines().filter_map(|line| line.split("verified-hash=").nth(1)).collect();
    assert_eq!(hashes.len(), 8);
    assert!(hashes.iter().all(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())));
    assert!(hashes.iter().any(|hash| hash.bytes().any(|byte| byte != b'0')));
}

#[test]
fn movement_guest_runs_the_shared_case_and_accepts_keyboard_inputs() {
    use std::{io::Write, process::Stdio};
    let executions = if cfg!(feature = "runtime") { vec!["manual", "runtime"] } else { vec!["manual"] };
    for execution in executions {
        let mut child = Command::new(env!("CARGO_BIN_EXE_contime-wasm-example"))
            .args([
                "--domain",
                "client-movement",
                "--execution",
                execution,
                "--clients",
                "4",
                "--workers",
                if execution == "runtime" { "2" } else { "1" },
                "--client-spacing",
                "9",
                "--movement-speed",
                "10",
                "--input-hz",
                "30",
                "--average-latency-ms",
                "100",
                "--jitter-ms",
                "100",
                "--trail-ms",
                "200",
                "--retention-ms",
                "1000",
                "--duration",
                "1",
                "--metrics-stdio",
                "--control-stdin",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"d").unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let samples: Vec<_> = String::from_utf8(output.stdout).unwrap().lines().map(|s| Sample::decode(s).unwrap()).collect();
        let last = samples.last().unwrap();
        let movement = last.movement.as_ref().unwrap();
        assert!(last.done);
        assert_eq!(last.metrics.outstanding, 0);
        assert_eq!(movement.keyboard_inputs, 1);
        assert_eq!((movement.client_spacing, movement.movement_speed), (9, 10));
        assert!((9.0..11.0).contains(&movement.center[0]));
        assert_eq!(movement.positions.len(), 4);
        assert!(movement.late > 0);
        assert!(movement.updates > 1);
        assert_eq!(last.reading.as_ref().unwrap().workers.len(), if execution == "runtime" { 2 } else { 1 });
    }
}
