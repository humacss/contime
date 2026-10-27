//! Terminal input belongs to a reader thread, never to event processing.

use crossbeam_channel::{bounded, select, Receiver, Sender, TryRecvError};
use ratatui::crossterm::event::{self, Event, KeyEvent};
use std::{
    io,
    thread::{self, JoinHandle},
    time::Duration,
};

pub(crate) const QUEUE_CAPACITY: usize = 64;
const INPUT_WAIT: Duration = Duration::from_millis(25);

/// The input owner must be dropped before restoring terminal raw mode.
/// Only this thread calls crossterm's poll/read while the dashboard is running.
pub(crate) struct Input {
    events: Receiver<io::Result<KeyEvent>>,
    stop: Sender<()>,
    reader: Option<JoinHandle<()>>,
}
impl Input {
    pub fn start() -> io::Result<Self> {
        Self::spawn(|timeout| {
            if event::poll(timeout)? {
                if let Event::Key(key) = event::read()? {
                    return Ok(Some(key));
                }
            }
            Ok(None)
        })
    }
    fn spawn(mut source: impl FnMut(Duration) -> io::Result<Option<KeyEvent>> + Send + 'static) -> io::Result<Self> {
        let (send, events) = bounded(QUEUE_CAPACITY);
        let (stop, cancelled) = bounded(1);
        let reader = thread::Builder::new().name("example-terminal-input".into()).spawn(move || loop {
            if !matches!(cancelled.try_recv(), Err(TryRecvError::Empty)) {
                break;
            }
            let message = match source(INPUT_WAIT) {
                Ok(Some(key)) => Ok(key),
                Ok(None) => continue,
                Err(error) => Err(error),
            };
            let failed = message.is_err();
            // Cancellation also interrupts a send when key repeat fills the queue.
            select! {
                recv(cancelled) -> _ => break,
                send(send, message) -> result => { if result.is_err() { break; } }
            }
            if failed {
                break;
            }
        })?;
        Ok(Self { events, stop, reader: Some(reader) })
    }
    /// Nonblocking in-memory receive: no terminal calls on the caller's thread.
    pub fn try_key(&self) -> io::Result<Option<KeyEvent>> {
        match self.events.try_recv() {
            Ok(result) => result.map(Some),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(io::Error::new(io::ErrorKind::BrokenPipe, "Terminal input reader stopped")),
        }
    }
}
impl Drop for Input {
    fn drop(&mut self) {
        let _ = self.stop.try_send(());
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::Instant,
    };
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn receive(input: &Input) -> io::Result<KeyEvent> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match input.try_key()? {
                Some(key) => return Ok(key),
                None => assert!(Instant::now() < deadline),
            }
            thread::sleep(Duration::from_millis(1));
        }
    }
    struct Lifetime(Arc<AtomicBool>);
    impl Drop for Lifetime {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Release);
        }
    }
    #[test]
    fn keys_arrive_in_order_from_another_thread_and_source_errors_are_visible() {
        let main = thread::current().id();
        let (send, source) = unbounded::<io::Result<KeyEvent>>();
        let input = Input::spawn(move |timeout| {
            assert_ne!(thread::current().id(), main);
            match source.recv_timeout(timeout) {
                Ok(value) => value.map(Some),
                Err(_) => Ok(None),
            }
        })
        .unwrap();
        send.send(Ok(key(KeyCode::End))).unwrap();
        send.send(Ok(key(KeyCode::Char('q')))).unwrap();
        send.send(Err(io::Error::other("input failed"))).unwrap();
        assert_eq!(receive(&input).unwrap().code, KeyCode::End);
        assert_eq!(receive(&input).unwrap().code, KeyCode::Char('q'));
        assert_eq!(receive(&input).unwrap_err().to_string(), "input failed");
    }
    #[test]
    fn idle_reader_is_joined_without_a_key_and_can_be_started_again() {
        for _ in 0..4 {
            let alive = Arc::new(AtomicBool::new(true));
            let lifetime = Lifetime(alive.clone());
            let (ready, started) = unbounded();
            let (_sender, source) = unbounded::<KeyEvent>();
            let input = Input::spawn(move |timeout| {
                let _keep_alive = &lifetime;
                let _ = ready.try_send(());
                Ok(source.recv_timeout(timeout).ok())
            })
            .unwrap();
            started.recv_timeout(Duration::from_secs(2)).unwrap();
            let start = Instant::now();
            drop(input);
            assert!(start.elapsed() < Duration::from_secs(1));
            assert!(!alive.load(Ordering::Acquire), "source thread was not joined");
        }
    }
    #[test]
    fn a_full_key_queue_does_not_block_shutdown() {
        let alive = Arc::new(AtomicBool::new(true));
        let lifetime = Lifetime(alive.clone());
        let input = Input::spawn(move |_| {
            let _keep_alive = &lifetime;
            Ok(Some(key(KeyCode::Down)))
        })
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while input.events.len() < QUEUE_CAPACITY {
            assert!(Instant::now() < deadline);
            thread::yield_now();
        }
        let start = Instant::now();
        drop(input);
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(!alive.load(Ordering::Acquire));
    }
}
