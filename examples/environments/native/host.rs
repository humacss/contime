//! Native process lifecycle; support supplies the case and application loop.
use std::sync::{atomic::AtomicBool, Arc};
type Failure = Box<dyn std::error::Error>;
pub fn run<R>(
    headless: bool,
    execute: impl FnOnce(Option<&mut ratatui::DefaultTerminal>, &Arc<AtomicBool>) -> Result<R, Failure>,
) -> Result<R, Failure> {
    let cancelled = Arc::new(AtomicBool::new(false));
    #[cfg(not(test))]
    {
        let signal = Arc::clone(&cancelled);
        ctrlc::set_handler(move || signal.store(true, std::sync::atomic::Ordering::Relaxed))?;
    }
    if headless {
        execute(None, &cancelled)
    } else {
        ratatui::run(|terminal| execute(Some(terminal), &cancelled))
    }
}
