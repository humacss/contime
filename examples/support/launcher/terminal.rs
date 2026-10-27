//! Host terminal presentation for metrics computed inside a WASI guest.
use crate::support::{
    monitored_wire::{Sample, MAX_LINE_BYTES},
    Settings,
};
use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender};
use std::{
    io::{self, BufRead, BufReader, IsTerminal, Read},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const MAX_RECORD_BYTES: u64 = MAX_LINE_BYTES as u64 + 2;
const STDERR_TAIL_BYTES: usize = 8 * 1024;
type Diagnostics = Arc<Mutex<Vec<u8>>>;
type Failure = Box<dyn std::error::Error>;

pub(super) fn enabled(settings: &Settings) -> bool {
    !settings.headless && io::stdin().is_terminal() && io::stdout().is_terminal()
}

// Drop disconnects blocked sends and closes the pipe by reaping the guest before
// joining the reader. No worker or terminal is left behind on an error path.
struct Guest {
    child: Child,
    samples: Option<Receiver<Result<Sample, String>>>,
    reader: Option<JoinHandle<()>>,
    stderr_reader: Option<JoinHandle<()>>,
    diagnostics: Diagnostics,
}
impl Guest {
    fn spawn(mut command: Command) -> io::Result<Self> {
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut guest = Self {
            child: command.spawn()?,
            samples: None,
            reader: None,
            stderr_reader: None,
            diagnostics: Arc::new(Mutex::new(Vec::new())),
        };
        let stdout = guest.child.stdout.take().expect("piped guest stdout");
        let (sender, receiver) = bounded(2);
        guest.samples = Some(receiver);
        guest.reader = Some(thread::Builder::new().name("wasi-metrics".into()).spawn(move || read_samples(stdout, sender))?);
        let stderr = guest.child.stderr.take().expect("piped guest stderr");
        let diagnostics = guest.diagnostics.clone();
        guest.stderr_reader =
            Some(thread::Builder::new().name("wasi-diagnostics".into()).spawn(move || read_diagnostics(stderr, diagnostics))?);
        Ok(guest)
    }
}
impl Drop for Guest {
    fn drop(&mut self) {
        self.samples.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }
}

fn read_diagnostics(mut input: impl Read, diagnostics: Diagnostics) {
    let mut buffer = [0; 1024];
    loop {
        match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                let mut tail = diagnostics.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let excess = (tail.len() + count).saturating_sub(STDERR_TAIL_BYTES);
                tail.drain(..excess);
                tail.extend_from_slice(&buffer[..count]);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

fn with_diagnostics(error: Failure, diagnostics: &Diagnostics) -> Failure {
    let tail = diagnostics.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let text = String::from_utf8_lossy(&tail);
    if text.trim().is_empty() {
        error
    } else {
        format!("{error}\nWASI host diagnostics (last 8 KiB):\n{}", text.trim()).into()
    }
}

fn read_samples(input: impl Read, sender: Sender<Result<Sample, String>>) {
    let mut input = BufReader::new(input);
    loop {
        let mut line = Vec::new();
        let read = input.by_ref().take(MAX_RECORD_BYTES + 1).read_until(b'\n', &mut line);
        let record = match read {
            Ok(0) => return,
            Ok(_) if line.len() as u64 > MAX_RECORD_BYTES => Err("WASI metrics record exceeds protocol limit".into()),
            Ok(_) if !line.ends_with(b"\n") => Err("Incomplete WASI metrics record".into()),
            Ok(_) => String::from_utf8(line)
                .map_err(|_| "WASI metrics must be UTF-8".into())
                .and_then(|line| Sample::decode(line.trim_end_matches(['\r', '\n']))),
            Err(error) => Err(format!("Could not read WASI metrics: {error}")),
        };
        let failed = record.is_err();
        if sender.send(record).is_err() || failed {
            return;
        }
    }
}

struct Outcome {
    metrics: Option<Sample>,
    completed: bool,
}

// The callback keeps input handling and presentation on the main thread. Tests
// exercise process/transport cleanup without entering terminal raw mode.
fn drive(
    guest: &mut Guest,
    cancelled: &AtomicBool,
    mut present: impl FnMut(Option<&Sample>) -> io::Result<bool>,
) -> Result<Outcome, Failure> {
    let mut latest = None;
    let mut done = false;
    loop {
        if cancelled.load(Ordering::Acquire) || present(latest.as_ref())? {
            return Ok(Outcome { metrics: latest, completed: false });
        }
        match guest.samples.as_ref().expect("metrics receiver").recv_timeout(Duration::from_millis(10)) {
            Ok(sample) => {
                let sample = sample.map_err(|error| format!("Invalid WASI metrics: {error}"))?;
                if done {
                    return Err("WASI emitted metrics after its final record".into());
                }
                done = sample.done;
                latest = Some(sample);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                if let Some(status) = guest.child.try_wait()? {
                    if !status.success() {
                        return Err(format!("WASI host exited with {status}").into());
                    }
                    if !done {
                        return Err("WASI exited without a final completed metrics record".into());
                    }
                    return Ok(Outcome { metrics: latest, completed: true });
                }
                if !done {
                    return Err("WASI closed its output without a final completed metrics record".into());
                }
                // EOF can precede process exit. Keep checking cancellation and
                // avoid spinning on a disconnected channel during host cleanup.
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

struct WorkerDisplay {
    input: Option<crate::support::ui::input::Input>,
    terminal: Option<crate::support::TerminalGuard>,
    view: crate::support::ui::workers::WorkerView<crate::definition::Time>,
    snapshots: crate::support::monitored_ui::SnapshotView,
    reading_at: Option<Duration>,
    controls: Option<std::process::ChildStdin>,
    stopping: bool,
    movement: bool,
    next_draw: std::time::Instant,
}
impl WorkerDisplay {
    fn new(controls: Option<std::process::ChildStdin>, movement: bool) -> io::Result<Self> {
        let terminal = crate::support::TerminalGuard::start()?;
        let input = crate::support::ui::input::Input::start()?;
        Ok(Self {
            controls,
            movement,
            stopping: false,
            input: Some(input),
            terminal: Some(terminal),
            view: Default::default(),
            snapshots: Default::default(),
            reading_at: None,
            next_draw: std::time::Instant::now(),
        })
    }
    fn present(&mut self, sample: Option<&crate::support::monitored_wire::Sample>) -> io::Result<bool> {
        if let Some(sample) = sample {
            if let Some(reading) = &sample.reading {
                // Repeated frames may carry the same reading. Do not reset the
                // cumulative-rate baseline by reinstalling an equal-clock sample.
                if self.reading_at.is_none_or(|at| reading.completed_at > at) {
                    self.reading_at = Some(reading.completed_at);
                    self.view.set_reading(Arc::clone(reading));
                }
            }
            if std::time::Instant::now() >= self.next_draw {
                if let Some(movement) = &sample.movement {
                    self.terminal
                        .as_mut()
                        .expect("active terminal")
                        .terminal
                        .draw(|frame| crate::support::ui::movement::render(frame, &mut self.view, movement, sample.metrics.elapsed))?;
                } else {
                    crate::support::monitored_ui::draw_sample(
                        &mut self.terminal.as_mut().expect("active terminal").terminal,
                        &mut self.view,
                        sample,
                        &mut self.snapshots,
                    )?;
                }
                self.next_draw = std::time::Instant::now() + Duration::from_millis(50);
            }
        }
        if self.movement {
            if sample.is_some_and(|s| s.done) {
                return Ok(false);
            }
            use ratatui::crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
            use std::io::Write;
            let input = self.input.as_ref().expect("active input");
            for _ in 0..64 {
                let Some(key) = input.try_key()? else { break };
                if key.kind == KeyEventKind::Release || self.stopping {
                    continue;
                }
                let byte = match key.code {
                    KeyCode::Char('w') | KeyCode::Up => b'w',
                    KeyCode::Char('a') | KeyCode::Left => b'a',
                    KeyCode::Char('s') | KeyCode::Down => b's',
                    KeyCode::Char('d') | KeyCode::Right => b'd',
                    KeyCode::Char(' ') => b' ',
                    KeyCode::Char('q') | KeyCode::Esc => b'q',
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => b'q',
                    _ => continue,
                };
                if let Some(pipe) = &mut self.controls {
                    pipe.write_all(&[byte])?;
                }
                self.stopping = byte == b'q';
            }
            return Ok(false);
        }
        let count = sample.map_or(0, |s| s.snapshots.len());
        let hashes = sample.is_some_and(|s| s.snapshots.first().is_some_and(|s| s.produced_hash.is_some()));
        crate::support::monitored_ui::quit(self.input.as_ref().expect("active input"), &mut self.view, &mut self.snapshots, count, hashes)
    }
}

pub(super) fn run(command: Command, cancelled: &AtomicBool, movement: bool) -> Result<(), Failure> {
    let mut guest = Guest::spawn(command)?;
    let mut display = WorkerDisplay::new(guest.child.stdin.take(), movement)?;
    let diagnostics = guest.diagnostics.clone();
    let result = drive(&mut guest, cancelled, |metrics| display.present(metrics));
    // Join both readers before using diagnostics; restore the terminal before
    // propagating a failure so guest errors remain visible to the caller.
    drop(guest);
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(error) => {
            drop(display);
            return Err(with_diagnostics(error, &diagnostics));
        }
    };
    if let Some(metrics) = outcome.metrics {
        display.input.take();
        display.terminal.take();
        println!("{}", metrics.metrics.text());
        if let Some(movement) = &metrics.movement {
            println!("Validated {} delivered inputs; {} reordered arrivals; {} corrected histories; {} rendering updates; {} keyboard inputs delivered",
                crate::support::number(movement.submitted),crate::support::number(movement.late),crate::support::number(movement.corrections),crate::support::number(movement.updates),crate::support::number(movement.keyboard_inputs));
        }
    }
    drop(display);
    if !outcome.completed {
        println!("WASI stopped by user.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn transport(bytes: Vec<u8>) -> Result<Sample, String> {
        let (send, receive) = bounded(2);
        read_samples(Cursor::new(bytes), send);
        receive.recv().unwrap()
    }

    #[test]
    fn malformed_truncated_and_oversized_records_fail_without_unbounded_buffers() {
        assert!(transport(b"garbage\n".to_vec()).is_err());
        assert!(transport(b"partial".to_vec()).err().unwrap().contains("Incomplete"));
        assert!(transport(vec![b'x'; MAX_RECORD_BYTES as usize + 1]).err().unwrap().contains("protocol limit"));
        assert!(transport(vec![255, b'\n']).err().unwrap().contains("UTF-8"));
    }

    #[cfg(unix)]
    fn shell(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        command
    }

    #[cfg(unix)]
    #[test]
    fn early_exit_and_decoder_failures_are_errors_and_children_are_reaped() {
        for script in ["exit 0", "printf 'bad\\n'; exec sleep 30"] {
            let mut guest = Guest::spawn(shell(script)).unwrap();
            assert!(drive(&mut guest, &AtomicBool::new(false), |_| Ok(false)).is_err());
            let pid = guest.child.id();
            drop(guest);
            assert!(!Command::new("kill").args(["-0", &pid.to_string()]).stderr(Stdio::null()).status().unwrap().success());
        }
    }

    #[cfg(unix)]
    fn emitting(script: &str, done: bool) -> Command {
        let sample = Sample {
            metrics: crate::support::monitored_metrics::Metrics::default(),
            reading: None,
            movement: None,
            snapshot_count: 0,
            snapshots: Vec::new(),
            done,
        }
        .encode();
        let mut command = shell(script);
        command.arg("metrics-test").arg(sample);
        command
    }

    #[cfg(unix)]
    #[test]
    fn completion_requires_both_a_final_record_and_successful_host_exit() {
        for (script, done, success) in [
            ("printf '%s\\n' \"$1\"", true, true),
            ("printf '%s\\n' \"$1\"; exit 7", true, false),
            ("printf '%s\\n' \"$1\"", false, false),
            ("printf '%s\\n%s\\n' \"$1\" \"$1\"", true, false),
        ] {
            let mut guest = Guest::spawn(emitting(script, done)).unwrap();
            let result = drive(&mut guest, &AtomicBool::new(false), |_| Ok(false));
            assert_eq!(result.is_ok(), success, "{script}");
            if let Ok(outcome) = result {
                assert!(outcome.completed);
                assert!(outcome.metrics.is_some());
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_full_sample_queue_does_not_block_cleanup() {
        let guest = Guest::spawn(emitting("while :; do printf '%s\\n' \"$1\"; done", false)).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while guest.samples.as_ref().unwrap().len() < 2 {
            assert!(std::time::Instant::now() < deadline, "guest did not fill the bounded queue");
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(guest.samples.as_ref().unwrap().capacity(), Some(2));
        // A reader blocked on its third send is released by Guest::drop.
        let pid = guest.child.id();
        drop(guest);
        assert!(!Command::new("kill").args(["-0", &pid.to_string()]).stderr(Stdio::null()).status().unwrap().success());
    }

    #[cfg(unix)]
    #[test]
    fn fatal_host_stderr_survives_cleanup_with_a_bounded_tail() {
        let script = r#"i=0; while [ "$i" -lt 3000 ]; do printf 'discarded prefix\n' >&2; i=$((i + 1)); done; printf 'fatal: missing guest module\n' >&2; exit 7"#;
        let mut guest = Guest::spawn(shell(script)).unwrap();
        let diagnostics = guest.diagnostics.clone();
        let result = drive(&mut guest, &AtomicBool::new(false), |_| Ok(false));
        assert!(result.is_err());
        drop(guest);
        let error = with_diagnostics(result.err().unwrap(), &diagnostics).to_string();
        assert!(error.contains("fatal: missing guest module"));
        assert_eq!(diagnostics.lock().unwrap().len(), STDERR_TAIL_BYTES);
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_and_presentation_failure_stop_and_join_the_guest() {
        for cancelled in [false, true] {
            let mut guest = Guest::spawn(shell("exec sleep 30")).unwrap();
            let result = drive(&mut guest, &AtomicBool::new(cancelled), |_| Err(io::Error::other("display failed")));
            if cancelled {
                assert!(!result.ok().unwrap().completed);
            } else {
                assert!(result.is_err());
            }
            let pid = guest.child.id();
            drop(guest);
            assert!(!Command::new("kill").args(["-0", &pid.to_string()]).stderr(Stdio::null()).status().unwrap().success());
        }
    }
}

#[cfg(test)]
mod worker_transport_tests {
    use super::*;
    #[test]
    fn obsolete_snapshot_protocol_is_rejected() {
        assert!(Sample::decode("contime-live-v4\t0").is_err());
    }
}
