//! Bounded child-process transport. The selected workload owns all simulation.
use crate::{
    bridge::Bridge,
    protocol::{self, Frame},
};
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread::{self, JoinHandle},
};

pub struct Source {
    child: Child,
    records: Option<Receiver<Result<Frame, String>>>,
    reader: Option<JoinHandle<()>>,
    diagnostics: Option<JoinHandle<()>>,
    tail: Arc<Mutex<Vec<u8>>>,
    final_seen: bool,
    eof: bool,
    stopping: bool,
}
impl Source {
    pub fn spawn(mut command: Command) -> io::Result<Self> {
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut source = Self {
            child: command.spawn()?,
            records: None,
            reader: None,
            diagnostics: None,
            tail: Arc::default(),
            final_seen: false,
            eof: false,
            stopping: false,
        };
        let stdout = source.child.stdout.take().ok_or_else(|| io::Error::other("Missing workload output pipe"))?;
        let (sender, receiver) = mpsc::sync_channel(2);
        source.records = Some(receiver);
        source.reader = Some(thread::Builder::new().name("graphics-reports".into()).spawn(move || {
            let mut input = BufReader::new(stdout);
            loop {
                let mut bytes = Vec::new();
                let record = match input.by_ref().take(protocol::MAX_LINE_BYTES as u64 + 2).read_until(b'\n', &mut bytes) {
                    Ok(0) => break,
                    Ok(_) if bytes.len() > protocol::MAX_LINE_BYTES + 1 => Err("Movement report exceeds protocol limit".into()),
                    Ok(_) if !bytes.ends_with(b"\n") => Err("Truncated movement report".into()),
                    Ok(_) => String::from_utf8(bytes)
                        .map_err(|e| e.to_string())
                        .and_then(|line| protocol::decode(line.trim_end_matches(['\r', '\n']))),
                    Err(error) => Err(error.to_string()),
                };
                let failed = record.is_err();
                if sender.send(record).is_err() || failed {
                    break;
                }
            }
        })?);
        let stderr = source.child.stderr.take().ok_or_else(|| io::Error::other("Missing workload diagnostic pipe"))?;
        let tail = source.tail.clone();
        source.diagnostics = Some(thread::Builder::new().name("graphics-diagnostics".into()).spawn(move || {
            let mut input = stderr;
            let mut buffer = [0; 1024];
            loop {
                match input.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        let mut tail = tail.lock().unwrap_or_else(|p| p.into_inner());
                        let excess = (tail.len() + count).saturating_sub(8192);
                        tail.drain(..excess);
                        tail.extend_from_slice(&buffer[..count]);
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        })?);
        Ok(source)
    }
    pub fn input(&mut self, key: u8) -> io::Result<()> {
        if self.stopping {
            return Ok(());
        }
        if let Some(input) = self.child.stdin.as_mut() {
            input.write_all(&[key])?;
            input.flush()?;
        }
        Ok(())
    }
    pub fn stop(&mut self) -> io::Result<()> {
        self.input(b'q')?;
        self.stopping = true;
        Ok(())
    }
    pub fn poll(&mut self, bridge: &Bridge) -> Result<bool, String> {
        // Two buffered records per frame, never an unbounded drain loop.
        for _ in 0..2 {
            match self.records.as_ref().ok_or("Workload transport closed")?.try_recv() {
                Ok(Ok(frame)) => {
                    if self.final_seen {
                        return Err("Workload sent data after its final report".into());
                    }
                    bridge.publish(frame.movement)?;
                    self.final_seen = frame.done;
                }
                Ok(Err(error)) => return Err(error),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.eof = true;
                    break;
                }
            }
        }
        if self.eof
            && let Some(status) = self.child.try_wait().map_err(|e| e.to_string())?
        {
            if !status.success() || !self.final_seen {
                let tail = self.tail.lock().unwrap_or_else(|p| p.into_inner());
                return Err(format!(
                    "Workload exited {status}; validated final report: {}\n{}",
                    self.final_seen,
                    String::from_utf8_lossy(&tail)
                ));
            }
            return Ok(true);
        }
        Ok(false)
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        self.records.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.diagnostics.take() {
            let _ = reader.join();
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    #[test]
    fn successful_exit_without_final_validation_is_rejected() {
        let mut command = Command::new("sh");
        command.args(["-c", "exit 0"]);
        let mut source = Source::spawn(command).unwrap();
        let start = Instant::now();
        loop {
            match source.poll(&Bridge::default()) {
                Err(error) => {
                    assert!(error.contains("validated final report: false"));
                    break;
                }
                Ok(_) => {
                    assert!(start.elapsed() < Duration::from_secs(3));
                    thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }
    #[test]
    fn dropping_a_silent_workload_reaps_it_promptly() {
        let mut command = Command::new("sh");
        command.args(["-c", "exec sleep 60"]);
        let source = Source::spawn(command).unwrap();
        let start = Instant::now();
        drop(source);
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    fn validated_child(exit_code: i32) -> Source {
        let report = serde_json::json!({"done": true, "movement": crate::animation::tests::scene(0.0)});
        let payload = format!("contime-workers-v2\t{report}");
        let mut command = Command::new("sh");
        // Pass the fixture as an argument, never interpolate it into shell code.
        command.args(["-c", "printf '%s\\n' \"$1\"; exit \"$2\"", "fixture", &payload, &exit_code.to_string()]);
        Source::spawn(command).unwrap()
    }
    #[test]
    fn final_report_requires_a_successful_workload_exit() {
        for code in [0, 7] {
            let mut source = validated_child(code);
            let start = Instant::now();
            let result = loop {
                match source.poll(&Bridge::default()) {
                    Ok(false) => {
                        assert!(start.elapsed() < Duration::from_secs(3));
                        thread::sleep(Duration::from_millis(1));
                    }
                    other => break other,
                }
            };
            assert_eq!(result.is_ok(), code == 0, "{result:?}");
        }
    }
}
