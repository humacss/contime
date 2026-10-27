//! Terminal presentation only. Settings and ConTime behavior stay outside UI.
pub(crate) mod input;
use std::io;
/// Restores the terminal on every return path, including early errors.
pub struct TerminalGuard {
    pub terminal: ratatui::DefaultTerminal,
}
impl TerminalGuard {
    pub fn start() -> io::Result<Self> {
        match ratatui::try_init() {
            Ok(terminal) => Ok(Self { terminal }),
            Err(error) => {
                ratatui::restore();
                Err(error)
            }
        }
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

pub mod workers;

pub mod movement;
