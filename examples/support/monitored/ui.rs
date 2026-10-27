//! Example-owned presentation of worker readings and independent snapshot checks.
use crate::definition::{Example, Time};
pub use crate::support::monitored_metrics::Metrics;
use crate::support::{
    monitored_metrics::SnapshotPair,
    ui::{
        input::Input,
        workers::{Scroll, WorkerView},
    },
};
use ratatui::{
    crossterm::event::{KeyCode, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout},
    widgets::{Block, Borders, Paragraph, Row, Table},
    DefaultTerminal, Frame,
};
use std::{io, time::Duration};
pub struct SnapshotView {
    offset: usize,
    page: usize,
    focus: bool,
}
impl Default for SnapshotView {
    fn default() -> Self {
        Self { offset: 0, page: 1, focus: true }
    }
}
fn digest(hash: Option<[u8; 32]>, full: bool) -> String {
    hash.map(|hash| hash.iter().take(if full { 32 } else { 8 }).map(|byte| format!("{byte:02x}")).collect()).unwrap_or_else(|| "-".into())
}
#[allow(clippy::too_many_arguments)]
pub fn draw<M: Example>(
    terminal: &mut DefaultTerminal,
    view: &mut WorkerView<Time>,
    now: Duration,
    metrics: &Metrics,
    expected: &[M::State],
    verified: &[M::State],
    snapshots: &mut SnapshotView,
) -> io::Result<()> {
    let hashes = M::hash(&M::State::default()).is_some();
    let count = if hashes { expected.len().min(verified.len()) } else { 0 };
    terminal.draw(|frame| {
        render(frame, view, now, metrics, snapshots, count, count, hashes, M::VALUE_LABEL, |i| {
            SnapshotPair::new::<M>(i, &expected[i], &verified[i])
        })
    })?;
    Ok(())
}
pub fn draw_sample(
    terminal: &mut DefaultTerminal,
    view: &mut WorkerView<Time>,
    sample: &crate::support::monitored_wire::Sample,
    snapshots: &mut SnapshotView,
) -> io::Result<()> {
    let hashes = sample.snapshots.first().is_some_and(|s| s.produced_hash.is_some());
    let count = if hashes { sample.snapshots.len() } else { 0 };
    terminal.draw(|frame| {
        render(frame, view, sample.metrics.elapsed, &sample.metrics, snapshots, count, sample.snapshot_count, hashes, "Value", |i| {
            sample.snapshots[i].clone()
        })
    })?;
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn render(
    frame: &mut Frame<'_>,
    view: &mut WorkerView<Time>,
    now: Duration,
    metrics: &Metrics,
    snapshots: &mut SnapshotView,
    available: usize,
    total: usize,
    hashes: bool,
    label: &str,
    mut row: impl FnMut(usize) -> SnapshotPair,
) {
    let height = frame.area().height.saturating_sub(16).min(10);
    let height = if available > 0 && height >= 6 { height } else { 0 };
    let areas = Layout::vertical([Constraint::Length(4), Constraint::Min(6), Constraint::Length(height)]).split(frame.area());
    frame.render_widget(Paragraph::new(metrics.text()), areas[0]);
    view.render(frame, areas[1], now);
    if height == 0 {
        return;
    }
    let detail = Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).split(areas[2]);
    snapshots.page = detail[0].height.saturating_sub(3).max(1) as usize;
    snapshots.offset = snapshots.offset.min(available.saturating_sub(snapshots.page));
    let rows = (snapshots.offset..available.min(snapshots.offset + snapshots.page)).map(|i| {
        let s = row(i);
        Row::new(vec![
            super::format::number(s.id),
            super::format::number(s.verified_count),
            super::format::number(s.produced_count),
            if hashes { digest(s.verified_hash, false) } else { super::format::number(s.verified_value) },
            if hashes { digest(s.produced_hash, false) } else { super::format::number(s.produced_value) },
        ])
    });
    let title = if available < total {
        format!(" Hash preview: first {} of {}; all snapshots validated ", super::format::number(available), super::format::number(total))
    } else if snapshots.focus {
        " Snapshots: arrows/pages scroll; Tab selects workers ".into()
    } else {
        " Snapshots: Tab selects snapshots ".into()
    };
    let table = Table::new(
        rows,
        [Constraint::Length(10), Constraint::Length(12), Constraint::Length(12), Constraint::Percentage(50), Constraint::Percentage(50)],
    )
    .header(Row::new(["Snapshot", "Verified #", "Produced #", "Last verified", "Latest produced"]))
    .block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(table, detail[0]);
    let s = row(snapshots.offset);
    let footer = if hashes {
        format!("Verified {}\nProduced {}", digest(s.verified_hash, true), digest(s.produced_hash, true))
    } else {
        format!("{label}\nVerified {} | Produced {}", super::format::number(s.verified_value), super::format::number(s.produced_value))
    };
    frame.render_widget(Paragraph::new(footer), detail[1]);
}
pub fn quit(
    input: &Input,
    view: &mut WorkerView<Time>,
    snapshots: &mut SnapshotView,
    snapshot_count: usize,
    has_snapshots: bool,
) -> io::Result<bool> {
    for _ in 0..64 {
        let Some(key) = input.try_key()? else { break };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        if has_snapshots && key.code == KeyCode::Tab {
            snapshots.focus = !snapshots.focus;
            continue;
        }
        let scroll = match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(true),
            KeyCode::Up => Some(Scroll::Up),
            KeyCode::Down => Some(Scroll::Down),
            KeyCode::PageUp => Some(Scroll::PageUp),
            KeyCode::PageDown => Some(Scroll::PageDown),
            KeyCode::Home => Some(Scroll::Home),
            KeyCode::End => Some(Scroll::End),
            _ => None,
        };
        if let Some(scroll) = scroll {
            if has_snapshots && snapshots.focus {
                snapshots.offset = match scroll {
                    Scroll::Up => snapshots.offset.saturating_sub(1),
                    Scroll::Down => snapshots.offset.saturating_add(1),
                    Scroll::PageUp => snapshots.offset.saturating_sub(snapshots.page),
                    Scroll::PageDown => snapshots.offset.saturating_add(snapshots.page),
                    Scroll::Home => 0,
                    Scroll::End => usize::MAX,
                }
                .min(snapshot_count.saturating_sub(snapshots.page));
            } else {
                view.scroll(scroll);
            }
        }
    }
    Ok(false)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hash_preview_is_explicit_and_only_visible_rows_are_formatted() {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 30)).unwrap();
        let mut view = WorkerView::default();
        let mut snapshots = SnapshotView::default();
        let mut reads = 0;
        terminal
            .draw(|frame| {
                render(frame, &mut view, Duration::ZERO, &Metrics::default(), &mut snapshots, 64, 1000, true, "", |i| {
                    reads += 1;
                    SnapshotPair {
                        id: i + 1,
                        produced_count: 2,
                        verified_count: 1,
                        produced_value: 3,
                        verified_value: 1,
                        produced_hash: Some([2; 32]),
                        verified_hash: Some([1; 32]),
                    }
                })
            })
            .unwrap();
        assert!(reads <= 8, "viewport formatted {reads} rows");
        let text = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect::<String>();
        assert!(text.contains("first 64 of 1,000"));
        assert!(text.contains(&"01".repeat(32)));
    }
}
