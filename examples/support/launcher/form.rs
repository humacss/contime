//! Shared launcher form presentation; starting examples stays outside the UI.
use super::super::ui::TerminalGuard;
use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
};
use std::{io, time::Duration};

pub struct Field {
    pub label: &'static str,
    pub help: &'static str,
    pub value: String,
    pub choices: Vec<String>,
    pub group: Option<&'static str>,
    pub visible: bool,
    pub allowed: Option<String>,
    pub decimal: bool,
    changed: bool,
    preferred_choice: Option<String>,
    initial: String,
    editing: bool,
}
impl Field {
    pub fn new(label: &'static str, help: &'static str, value: String, choices: Vec<String>) -> Self {
        Self {
            label,
            help,
            initial: value.clone(),
            value,
            choices,
            editing: false,
            group: None,
            visible: true,
            allowed: None,
            decimal: true,
            changed: false,
            preferred_choice: None,
        }
    }
    /// Record a user choice independently of temporary environment restrictions.
    pub fn remember_choice(&mut self) {
        self.preferred_choice = Some(self.value.clone());
    }
    /// Choices are ordered with the current default first. Untouched fields follow it.
    pub fn set_choices(&mut self, choices: Vec<String>) {
        if let Some(default) = choices.first() {
            self.initial = default.clone();
            self.value = self.preferred_choice.as_ref().filter(|value| choices.contains(value)).unwrap_or(default).clone();
        }
        self.choices = choices;
    }
    pub fn changed(&self) -> bool {
        self.changed || self.value != self.initial
    }
    pub fn update_default(&mut self, value: String) {
        if !self.changed() {
            self.initial = value.clone();
            self.value = value;
        }
    }
    fn adjust(&mut self, forward: bool, numeric: bool) {
        if self.choices.is_empty() {
            return;
        }
        if numeric {
            let maximum = self.choices.iter().any(|v| v == "Maximum");
            if self.value == "Maximum" {
                if !forward {
                    if let Some(value) = self.choices.iter().rev().find(|v| *v != "Maximum") {
                        self.value = value.clone();
                    }
                }
            } else if let Ok(current) = self.value.parse::<f64>() {
                let next = if forward {
                    self.choices.iter().find(|v| v.parse::<f64>().is_ok_and(|n| n > current))
                } else {
                    self.choices.iter().rev().find(|v| v.parse::<f64>().is_ok_and(|n| n < current))
                };
                if let Some(value) = next {
                    self.value = value.clone();
                } else if forward && maximum {
                    self.value = "Maximum".into();
                }
            } else if let Some(value) = self.choices.first() {
                self.value = value.clone();
            }
            self.editing = false;
            return;
        }
        let index = self.choices.iter().position(|v| *v == self.value);
        let index = match index {
            Some(i) if forward => (i + 1) % self.choices.len(),
            Some(i) => (i + self.choices.len() - 1) % self.choices.len(),
            None if forward => 0,
            None => self.choices.len() - 1,
        };
        self.value = self.choices[index].clone();
        self.editing = false;
    }
}

pub struct Form {
    pub fields: Vec<Field>,
    pub selected: usize,
    pub error: String,
    pub numeric: bool,
}
impl Form {
    pub fn new(fields: Vec<Field>, numeric: bool) -> Self {
        Self { fields, selected: 0, error: String::new(), numeric }
    }
    pub fn handle(&mut self, key: KeyEvent) {
        self.error.clear();
        let len = self.fields.len();
        if len == 0 {
            return;
        }
        let edited = matches!(
            key.code,
            KeyCode::Left | KeyCode::Right | KeyCode::Backspace | KeyCode::Char(' ') | KeyCode::Char('m') | KeyCode::Char('r')
        ) || matches!(key.code, KeyCode::Char(c) if self.numeric && (c.is_ascii_digit() || c == '.' && self.fields[self.selected].decimal));
        match key.code {
            KeyCode::Up | KeyCode::BackTab => self.move_selection(false),
            KeyCode::Down | KeyCode::Tab => self.move_selection(true),
            KeyCode::Left => self.fields[self.selected].adjust(false, self.numeric),
            KeyCode::Right | KeyCode::Char(' ') => self.fields[self.selected].adjust(true, self.numeric),
            KeyCode::Char('m') if self.fields[self.selected].choices.iter().any(|choice| choice == "Maximum") => {
                self.fields[self.selected].value = "Maximum".into();
                self.fields[self.selected].editing = false;
            }
            KeyCode::Char('r') => {
                let field = &mut self.fields[self.selected];
                field.value.clone_from(&field.initial);
                field.editing = false;
            }
            KeyCode::Char(c) if self.numeric && (c.is_ascii_digit() || c == '.' && self.fields[self.selected].decimal) => {
                let field = &mut self.fields[self.selected];
                if !field.editing {
                    field.value.clear();
                }
                field.editing = true;
                field.value.push(c);
            }
            KeyCode::Backspace if self.numeric => {
                let field = &mut self.fields[self.selected];
                field.value.pop();
                field.editing = true;
            }
            _ => {}
        }
        if edited {
            self.fields[self.selected].changed = true;
            if !self.numeric {
                self.fields[self.selected].remember_choice();
            }
        }
        if matches!(key.code, KeyCode::Up | KeyCode::Down | KeyCode::Tab | KeyCode::BackTab) {
            self.fields[self.selected].editing = false;
        }
    }
    pub fn focus(&mut self, index: usize) {
        self.selected = index;
        self.fields[index].editing = false;
    }
    fn move_selection(&mut self, forward: bool) {
        let len = self.fields.len();
        for _ in 0..len {
            self.selected = if forward { (self.selected + 1) % len } else { (self.selected + len - 1) % len };
            if self.fields[self.selected].visible {
                break;
            }
        }
    }
    pub fn run(
        &mut self,
        title: &str,
        action: &str,
        mut update: impl FnMut(&mut Self),
        mut validate: impl FnMut(&mut Self) -> Result<(), String>,
    ) -> io::Result<bool> {
        let mut terminal = TerminalGuard::start()?;
        loop {
            terminal.terminal.draw(|frame| self.render(frame, title, action))?;
            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            let Event::Key(key) = event::read()? else { continue };
            if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                continue;
            }
            if key.code == KeyCode::Esc || key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                return Ok(false);
            }
            if key.code == KeyCode::Enter {
                match validate(self) {
                    Ok(()) => return Ok(true),
                    Err(error) => self.error = error,
                }
            } else {
                self.handle(key);
                update(self);
            }
        }
    }
    fn display_value(&self, value: &str) -> String {
        if self.numeric {
            value.parse::<u64>().map(super::super::number).unwrap_or_else(|_| value.to_string())
        } else {
            value.to_string()
        }
    }
    pub(super) fn render(&self, frame: &mut ratatui::Frame, title: &str, action: &str) {
        let area = frame.area();
        if area.width < 80 || area.height < 24 {
            frame.render_widget(Paragraph::new("Resize the terminal to at least 80 × 24. Esc cancels."), area);
            return;
        }
        let parts = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length((self.fields.len() as u16 + 5).min(area.height.saturating_sub(17))),
            Constraint::Min(7),
            Constraint::Length(2),
            Constraint::Length(2),
        ])
        .margin(1)
        .split(area);
        frame.render_widget(
            Paragraph::new(format!("ConTime / {title}\nChoose your setup before starting ConTime."))
                .style(Style::default().fg(Color::Cyan)),
            parts[0],
        );
        let mut rows = Vec::new();
        let mut group = None;
        let mut selected_row = 0;
        for (i, field) in self.fields.iter().enumerate().filter(|(_, f)| f.visible) {
            if field.group != group {
                group = field.group;
                if let Some(group) = group {
                    rows.push(
                        Row::new([Cell::from(""), Cell::from(group), Cell::from(""), Cell::from("")])
                            .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    );
                }
            }
            if i == self.selected {
                selected_row = rows.len();
            }
            rows.push(
                Row::new([
                    Cell::from(if i == self.selected { " › " } else { "   " }),
                    Cell::from(field.label),
                    Cell::from(self.display_value(&field.value)),
                    Cell::from(field.choices.iter().position(|choice| choice == &field.value).map_or_else(
                        || format!("– / {}", field.choices.len()),
                        |index| format!("{} / {}", index + 1, field.choices.len()),
                    )),
                ])
                .style(if i == self.selected {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                }),
            );
        }
        let count = rows.len();
        let capacity = usize::from(parts[1].height.saturating_sub(2)).max(1);
        // Keep the selected field visible, together with its group heading when possible.
        let offset = selected_row.saturating_sub(capacity.saturating_sub(1));
        let mut state = TableState::default().with_offset(offset).with_selected(Some(selected_row));
        frame.render_stateful_widget(
            Table::new(rows, [Constraint::Length(3), Constraint::Length(27), Constraint::Min(20), Constraint::Length(9)]).block(
                Block::default()
                    .title(format!(" Configuration · rows {}–{} / {count} ", offset + 1, (offset + capacity).min(count)))
                    .borders(Borders::ALL),
            ),
            parts[1],
            &mut state,
        );
        let field = &self.fields[self.selected];
        let choices = field.choices.iter().map(|value| self.display_value(value)).collect::<Vec<_>>().join("  /  ");
        frame.render_widget(
            Paragraph::new(format!(
                "{}{}\n\n{}: {choices}",
                field.allowed.as_deref().unwrap_or(""),
                field.help,
                if self.numeric { "Common values" } else { "Choices" }
            ))
            .wrap(Wrap { trim: true })
            .block(Block::default().title(format!(" {} ", field.label)).borders(Borders::ALL)),
            parts[2],
        );
        frame.render_widget(Paragraph::new(self.error.as_str()).style(Style::default().fg(Color::Red)).wrap(Wrap { trim: true }), parts[3]);
        frame.render_widget(
            Paragraph::new(format!(
                "↑↓ / Tab select   ←→ change   {}r reset\nEnter {action}   Esc cancel",
                if self.numeric { "Type value   " } else { "" }
            )),
            parts[4],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    #[test]
    fn choice_navigation_wraps_and_reset_restores_initial_value() {
        let mut form = Form::new(vec![Field::new("Mode", "", "a".into(), vec!["a".into(), "b".into()])], false);
        form.handle(key(KeyCode::Left));
        assert_eq!(form.fields[0].value, "b");
        form.handle(key(KeyCode::Right));
        assert_eq!(form.fields[0].value, "a");
        form.handle(key(KeyCode::Right));
        form.handle(key(KeyCode::Char('r')));
        assert_eq!(form.fields[0].value, "a");
    }
    #[test]
    fn typing_replaces_value_and_refocusing_starts_a_new_edit() {
        let field = || Field::new("Budget", "", "1000".into(), vec!["1".into(), "1000".into()]);
        let mut form = Form::new(vec![field(), field()], true);
        for c in ['0', '.', '5'] {
            form.handle(key(KeyCode::Char(c)));
        }
        assert_eq!(form.fields[0].value, "0.5");
        form.handle(key(KeyCode::Tab));
        form.handle(key(KeyCode::BackTab));
        form.handle(key(KeyCode::Char('2')));
        assert_eq!(form.fields[0].value, "2");
    }
    #[test]
    fn form_renders_help_and_controls_at_minimum_size() {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        let form = Form::new(vec![Field::new("Environment", "Where the example runs", "Browser".into(), vec!["Browser".into()])], false);
        terminal.draw(|frame| form.render(frame, "Example launcher", "continue")).unwrap();
        let text: String = terminal.backend().buffer().content().iter().map(|cell| cell.symbol()).collect();
        for expected in ["Environment", "Where the example runs", "Enter continue", "Esc cancel"] {
            assert!(text.contains(expected), "{expected}");
        }
    }
    #[test]
    fn grouped_form_scrolls_to_last_setting_and_skips_hidden_fields() {
        let mut fields =
            (0..20).map(|_| Field::new("Setting", "Selected help", "1".into(), vec!["1".into(), "2".into()])).collect::<Vec<_>>();
        for (i, field) in fields.iter_mut().enumerate() {
            field.group = Some(if i < 3 { "ConTime" } else { "Client movement" });
        }
        fields[1].visible = false;
        fields[19].label = "Last setting";
        let mut form = Form::new(fields, true);
        form.handle(key(KeyCode::Down));
        assert_eq!(form.selected, 2);
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| form.render(frame, "Settings", "start")).unwrap();
        let text: String = terminal.backend().buffer().content().iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("ConTime"));
        form.selected = 19;
        terminal.draw(|frame| form.render(frame, "Settings", "start")).unwrap();
        let text: String = terminal.backend().buffer().content().iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("Last setting") && text.contains("Selected help"));
    }
    #[test]
    fn choosing_current_default_still_counts_as_an_explicit_edit() {
        let mut form = Form::new(vec![Field::new("Setting", "", "1".into(), vec!["1".into()])], true);
        form.handle(key(KeyCode::Right));
        form.fields[0].update_default("2".into());
        assert_eq!(form.fields[0].value, "1");
    }

    #[test]
    fn numeric_presets_use_nearest_value_and_put_maximum_after_highest_number() {
        let mut form = Form::new(vec![Field::new("Rate", "", "7".into(), vec!["1".into(), "10".into(), "Maximum".into()])], true);
        form.handle(key(KeyCode::Right));
        assert_eq!(form.fields[0].value, "10");
        form.handle(key(KeyCode::Right));
        assert_eq!(form.fields[0].value, "Maximum");
        form.handle(key(KeyCode::Right));
        assert_eq!(form.fields[0].value, "Maximum");
        form.handle(key(KeyCode::Left));
        assert_eq!(form.fields[0].value, "10");
    }
    #[test]
    fn numeric_custom_values_reset_backspace_and_render_numbered_maximum_preset() {
        let mut form = Form::new(vec![Field::new("Rate", "", "7".into(), vec!["1".into(), "10".into(), "Maximum".into()])], true);
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        for (code, value, indicator) in [
            (KeyCode::Char('r'), "7", "– / 3"),
            (KeyCode::Right, "10", "2 / 3"),
            (KeyCode::Right, "Maximum", "3 / 3"),
            (KeyCode::Left, "10", "2 / 3"),
            (KeyCode::Char('r'), "7", "– / 3"),
        ] {
            form.handle(key(code));
            assert_eq!(form.fields[0].value, value);
            terminal.draw(|frame| form.render(frame, "Counter", "start")).unwrap();
            let text: String = terminal.backend().buffer().content().iter().map(|cell| cell.symbol()).collect();
            assert!(text.contains(indicator));
        }
        form.handle(key(KeyCode::Backspace));
        assert!(form.fields[0].value.is_empty());
        for c in ['9', '7'] {
            form.handle(key(KeyCode::Char(c)));
        }
        assert_eq!(form.fields[0].value, "97");
        form.handle(key(KeyCode::Left));
        assert_eq!(form.fields[0].value, "10");
        form.handle(key(KeyCode::Left));
        form.handle(key(KeyCode::Left));
        assert_eq!(form.fields[0].value, "1");
    }
}
