use std::time::Duration;

use super::metrics::{Metrics, OrderingCheck};

fn milliseconds(value: Option<Duration>) -> String {
    value.map(|value| format!("{:.2} ms", value.as_secs_f64() * 1_000.0)).unwrap_or_else(|| "—".to_owned())
}

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;

pub struct Dashboard {
    pub values: Vec<i64>,
    pub verified_counts: Vec<u64>,
    pub hashes: Vec<[u8; 32]>,
    pub query_time: Duration,
    pub submitted: usize,
    pub reordered: usize,
    pub query_through: Duration,
    workers: usize,
    events_per_second: usize,
    selection: TableState,
}

impl Dashboard {
    pub fn new(counters: usize, workers: usize, events_per_second: usize) -> Self {
        Self {
            values: vec![0; counters],
            verified_counts: vec![0; counters],
            hashes: vec![[0; 32]; counters],
            query_time: Duration::ZERO,
            submitted: 0,
            reordered: 0,
            query_through: Duration::ZERO,
            workers,
            events_per_second,
            selection: TableState::default().with_selected(0),
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
            return false;
        }
        let selected = self.selection.selected().unwrap_or(0);
        let last = self.values.len().saturating_sub(1);
        let next = match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return true,
            KeyCode::Down | KeyCode::Char('j') => selected.saturating_add(1).min(last),
            KeyCode::Up | KeyCode::Char('k') => selected.saturating_sub(1),
            KeyCode::PageDown => selected.saturating_add(10).min(last),
            KeyCode::PageUp => selected.saturating_sub(10),
            KeyCode::Home => 0,
            KeyCode::End => last,
            _ => return false,
        };
        self.selection.select(Some(next));
        false
    }

    pub fn draw(&mut self, frame: &mut Frame, elapsed: Duration, metrics: &Metrics) {
        let area = frame.area();
        frame.render_widget(Block::default().style(Style::default().bg(Color::Rgb(20, 26, 36)).fg(Color::Gray)), area);
        if area.width < 96 || area.height < 17 {
            frame.render_widget(Paragraph::new("Enlarge the terminal to view counters.\nq / Esc / Ctrl-C to quit"), area);
            return;
        }
        let areas = Layout::vertical([Constraint::Length(8), Constraint::Min(5), Constraint::Length(4)]).split(area);
        let header = Paragraph::new(vec![
            Line::from(vec![
                Span::styled("ConTime", Style::default().fg(Color::LightCyan).add_modifier(Modifier::BOLD)),
                Span::raw("  Chain hash verification"),
            ]),
            Line::from(format!("{} counters   {} workers   20 Hz display", self.values.len(), self.workers)),
            Line::from(format!(
                "Event rate   target {}/s   actual {:.1}/s (run average)",
                self.events_per_second,
                metrics.produced_events as f64 / elapsed.as_secs_f64().max(0.001)
            )),
            Line::from(format!("Delivery   Submitted {}   Out-of-order arrivals {}", self.submitted, self.reordered)),
            Line::from(format!(
                "Events   Produced {}   Verified {}   Pending {}",
                metrics.produced_events, metrics.verified_events, metrics.pending
            )),
            Line::from(format!(
                "First-query delay   last {}   avg {}   max {}",
                milliseconds(metrics.last_delay),
                milliseconds(metrics.average_delay),
                milliseconds(metrics.max_delay)
            )),
            Line::from(format!(
                "Elapsed {:.1}s   Query {:.2} ms   Query through {:.3}s",
                elapsed.as_secs_f64(),
                self.query_time.as_secs_f64() * 1_000.0,
                self.query_through.as_secs_f64()
            )),
        ])
        .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(Color::DarkGray)));
        frame.render_widget(header, areas[0]);

        let rows = self.values.iter().enumerate().map(|(index, _)| {
            let check = &metrics.checks[index];
            let (ordering, color) = match check.ordering {
                OrderingCheck::NoEvents => ("No events", Color::DarkGray),
                OrderingCheck::Matches => ("Matches", Color::LightGreen),
            };
            Row::new([
                Cell::from(format!("Counter {}", index + 1)),
                Cell::from(check.produced_count.to_string()),
                Cell::from(check.verified_count.to_string()).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Cell::from(check.produced_count.saturating_sub(check.verified_count).to_string()).style(Style::default().fg(Color::Yellow)),
                Cell::from(check.expected_hash.map(short_hash).unwrap_or_else(|| "—".to_owned())),
                Cell::from(short_hash(check.produced_hash)).style(Style::default().fg(Color::LightCyan)),
                Cell::from(ordering).style(Style::default().fg(color)),
            ])
        });
        let table = Table::new(
            rows,
            [
                Constraint::Length(12),
                Constraint::Length(10),
                Constraint::Length(10),
                Constraint::Length(8),
                Constraint::Length(16),
                Constraint::Length(16),
                Constraint::Min(10),
            ],
        )
        .header(
            Row::new(["History", "Produced", "Verified", "Pending", "Last valid hash", "Latest produced", "Ordering"])
                .style(Style::default().fg(Color::LightCyan))
                .bottom_margin(1),
        )
        .block(Block::default().title(" Counters ").borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)))
        .row_highlight_style(Style::default().bg(Color::Rgb(40, 52, 68)))
        .highlight_symbol("› ");
        frame.render_stateful_widget(table, areas[1], &mut self.selection);

        let selected = self.selection.selected().unwrap_or(0);
        let check = &metrics.checks[selected];
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(format!(
                    "Counter {} valid:    {}",
                    selected + 1,
                    blake3::Hash::from_bytes(check.expected_hash.unwrap_or_default())
                )),
                Line::from(format!("Counter {} produced: {}", selected + 1, blake3::Hash::from_bytes(check.produced_hash))),
                Line::styled("↑↓ / j k scroll   Home / End   PgUp / PgDn   q / Esc / Ctrl-C quit", Style::default().fg(Color::LightCyan)),
                Line::from(format!(
                    "Reversed groups; queries use admitted prefixes. Oldest pending: {}",
                    milliseconds(metrics.oldest_pending)
                )),
            ]),
            areas[2],
        );
    }
}

fn short_hash(hash: [u8; 32]) -> String {
    // Display a prefix; comparisons always use all 32 bytes.
    hash[..6].iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn renders_values_and_scrolls_to_the_last_counter() {
        let mut dashboard = Dashboard::new(40, 2, 100);
        dashboard.values[39] = 123;
        assert!(!dashboard.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)));
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal.draw(|frame| dashboard.draw(frame, Duration::from_secs(2), super::super::metrics::Tracker::new(40).metrics())).unwrap();
        let text = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect::<String>();
        assert!(text.contains("ConTime"));
        assert!(text.contains("Counter 40"));
        assert!(text.contains("2 workers"));
        assert!(text.contains("Produced"));
        assert!(text.contains("Verified"));
        assert!(text.contains("First-query delay"));
        assert!(text.contains("Ordering"));
        assert!(text.contains("No events"));
        assert!(text.contains("target 100/s"));
    }

    #[test]
    fn latest_produced_and_last_valid_hashes_are_both_visible() {
        use super::super::metrics::{ProducedEvent, Tracker};
        use super::super::{CounterChanged, Time};
        let mut dashboard = Dashboard::new(1, 1, 100);
        let mut tracker = Tracker::new(1);
        let now = std::time::Instant::now();
        tracker
            .record(ProducedEvent { event: CounterChanged { event_id: 1, counter_ids: vec![1], time: Time(1), delta: 3 }, started: now });
        tracker.observe(&[0], &[[0; 32]], &[0], now).unwrap();
        let produced = blake3::Hash::from_bytes(tracker.metrics().checks[0].produced_hash).to_string();
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal.draw(|frame| dashboard.draw(frame, Duration::ZERO, tracker.metrics())).unwrap();
        let text = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect::<String>();
        assert!(text.contains("Last valid hash"));
        assert!(text.contains("Latest produced"));
        assert!(text.contains("valid:    000000000000"));
        assert!(text.contains(&produced));
    }

    #[test]
    fn small_terminals_and_quit_keys_are_supported() {
        let mut dashboard = Dashboard::new(1, 1, 10);
        let mut terminal = Terminal::new(TestBackend::new(20, 5)).unwrap();
        terminal.draw(|frame| dashboard.draw(frame, Duration::ZERO, super::super::metrics::Tracker::new(1).metrics())).unwrap();
        for key in [
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ] {
            assert!(dashboard.handle_key(key));
        }
    }
}
