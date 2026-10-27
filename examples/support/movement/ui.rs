//! Terminal map and input decoding; simulation and ConTime queries live elsewhere.
use crate::definition::Time;
use crate::movement_example::model::Direction;
use crate::support::{
    movement_report::Report,
    ui::{input::Input, workers::WorkerView},
};
use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    DefaultTerminal,
};
use std::{io, time::Duration};
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Quit,
    Turn(Direction),
}
pub fn command(key: KeyEvent) -> Option<Command> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => Some(Command::Quit),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(Command::Quit),
        KeyCode::Char('w') | KeyCode::Up => Some(Command::Turn(super::portable::direction("north").ok()?)),
        KeyCode::Char('s') | KeyCode::Down => Some(Command::Turn(super::portable::direction("south").ok()?)),
        KeyCode::Char('a') | KeyCode::Left => Some(Command::Turn(super::portable::direction("west").ok()?)),
        KeyCode::Char('d') | KeyCode::Right => Some(Command::Turn(super::portable::direction("east").ok()?)),
        KeyCode::Char(' ') => Some(Command::Turn(super::portable::direction("stop").ok()?)),
        _ => None,
    }
}
pub struct View<'a> {
    pub terminal: Option<&'a mut DefaultTerminal>,
    input: Option<Input>,
    pub workers: WorkerView<Time>,
}
impl<'a> View<'a> {
    pub fn new(terminal: Option<&'a mut DefaultTerminal>) -> io::Result<Self> {
        let input = terminal.as_ref().map(|_| Input::start()).transpose()?;
        Ok(Self { terminal, input, workers: WorkerView::default() })
    }
    pub fn commands(&self) -> io::Result<Vec<Command>> {
        let mut commands = Vec::new();
        if let Some(input) = &self.input {
            for _ in 0..64 {
                let Some(key) = input.try_key()? else { break };
                if let Some(c) = command(key) {
                    commands.push(c);
                }
            }
        }
        Ok(commands)
    }
    pub fn draw(&mut self, report: &Report, now: Duration) -> io::Result<()> {
        if let Some(terminal) = self.terminal.as_deref_mut() {
            terminal.draw(|frame| crate::support::ui::movement::render(frame, &mut self.workers, report, now))?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyboard_commands_choose_persistent_direction_and_ignore_release() {
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        assert_eq!(command(key(KeyCode::Char('w'))), Some(Command::Turn(Direction::North)));
        assert_eq!(command(key(KeyCode::Char(' '))), Some(Command::Turn(Direction::Stop)));
        assert_eq!(command(key(KeyCode::Char('q'))), Some(Command::Quit));
        let mut release = key(KeyCode::Char('d'));
        release.kind = KeyEventKind::Release;
        assert_eq!(command(release), None);
    }
}
