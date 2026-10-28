//! Terminal presentation of movement reports, independent of their execution host.
use crate::definition::Time;
use crate::support::{movement_report::Report, number, ui::workers::WorkerView};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Style},
    text::Span,
    widgets::{
        canvas::{Canvas, Points, Rectangle},
        Block, Borders, Paragraph,
    },
    Frame,
};
use std::time::Duration;
pub fn render(frame: &mut Frame<'_>, workers: &mut WorkerView<Time>, report: &Report, now: Duration) {
    let parts =
        Layout::vertical([Constraint::Length(3), Constraint::Min(8), Constraint::Length(9), Constraint::Length(2)]).split(frame.area());
    frame.render_widget(Paragraph::new(format!("Client movement | Clients {:>6} | Workers {:>4} | Average lag {:>8} ms ± {:>6} ms\nDelivered {:>20} | Network {:>12} | {:>12} reordered arrivals\nCorrected client histories {:>12} | positions from queried input history{}",number(report.clients),number(report.workers),number(report.average_latency_ms),number(report.jitter_ms),number(report.submitted),number(report.in_flight),number(report.late),number(report.corrections),if report.preview_limited {" | bounded visual preview"} else {""})),parts[0]);
    let [cx, cy] = report.center;
    let [half_x, half_y] = report.arena_half_extent;
    let map = Canvas::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" You @ ({cx:.1}, {cy:.1}) | cyan: queried path | amber: revised old path ")),
        )
        .x_bounds([-half_x - 2.0, half_x + 2.0])
        .y_bounds([-half_y - 2.0, half_y + 2.0])
        .paint(|ctx| {
            ctx.draw(&Rectangle { x: -half_x, y: -half_y, width: 2.0 * half_x, height: 2.0 * half_y, color: Color::DarkGray });
            for point in report.trails.iter().rev() {
                let base = if point.corrected {
                    [190.0, 120.0, 20.0]
                } else if point.current {
                    [0.0, 210.0, 230.0]
                } else {
                    [65.0, 75.0, 90.0]
                };
                let c = |i: usize| (base[i] * point.opacity.clamp(0.0, 1.0)) as u8;
                ctx.draw(&Points { coords: &[(point.position[0], point.position[1])], color: Color::Rgb(c(0), c(1), c(2)) });
            }
            for client in &report.positions {
                let [x, y] = client.position;
                let color = if client.id == 1 {
                    Color::Yellow
                } else if client.corrected {
                    Color::LightRed
                } else {
                    Color::Green
                };
                let label = if client.id == 1 { "@".to_string() } else { char::from(b'a' + ((client.id - 1) % 26) as u8).to_string() };
                ctx.print(x, y, Span::styled(label, Style::default().fg(color)));
            }
        });
    frame.render_widget(map, parts[1]);
    workers.render(frame, parts[2], now);
    frame.render_widget(Paragraph::new("WASD / arrows: choose direction   Space: stop   q / Esc / Ctrl-C: quit\nKeyboard inputs use the same delayed delivery as bots. Walls bound the shared arena; the whole arena stays in view."),parts[3]);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_map_displays_player_and_amber_corrected_paths() {
        let report:Report=serde_json::from_value(serde_json::json!({"clients":1,"workers":1,"client_spacing":4,"movement_speed":6,"average_latency_ms":50,"jitter_ms":25,
            "submitted":3,"in_flight":1,"late":1,"corrections":1,"updates":2,"keyboard_inputs":1,"center":[0.0,0.0],"arena_half_extent":[48.0,28.0],
            "positions":[{"id":1,"position":[0.0,0.0],"corrected":true}],"trails":[{"position":[-2.0,0.0],"corrected":true,"opacity":0.8,"current":false}],"preview_limited":false})).unwrap();
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 40)).unwrap();
        terminal.draw(|frame| render(frame, &mut WorkerView::default(), &report, Duration::from_secs(1))).unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("reordered arrivals") && text.contains("WASD"));
        assert!(buffer.content().iter().any(|c| c.symbol() == "@"));
        assert!(buffer.content().iter().any(|c| matches!(c.fg,Color::Rgb(r,g,b) if r>g&&g>b)));
    }
}
