use super::WorkerView;
use crate::support::format::number;
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Cell, Paragraph, Row, Table},
    Frame,
};
use std::{fmt::Display, time::Duration};

fn clipped_ascii(text: String, width: usize) -> String {
    // Summary lines can already contain an ellipsis from a clipped field.
    if text.len() <= width || text.chars().count() <= width {
        text
    } else {
        text.chars().take(width.saturating_sub(1)).chain(std::iter::once('…')).collect()
    }
}
fn slot(text: String, width: usize) -> String {
    format!("{:>width$}", clipped_ascii(text, width))
}
fn count_cell(value: impl Display, width: usize) -> String {
    clipped_ascii(number(value), width)
}
fn optional_count(value: Option<impl Display>) -> String {
    value.map_or_else(|| "-".into(), number)
}
fn rate(value: Option<f64>) -> String {
    value.map_or_else(|| "-".into(), |value| number(format!("{value:.0}")))
}
impl<T: Display> WorkerView<T> {
    /// Draws into a host-owned frame using a supplied elapsed clock.
    ///
    /// No collector is accessed. Age saturates at zero if the supplied clock
    /// precedes the reading. With counters enabled, per-worker rates remain visible in compact views;
    /// views below 120 columns stack totals and cumulative average rates on two lines per worker.
    /// Time targets
    /// appear at 140 columns. Very small areas display a resize hint.
    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, now: Duration) {
        if area.width < 48 || area.height < 12 {
            frame.render_widget(Paragraph::new("Enlarge terminal (48 x 12)"), area);
            return;
        }
        let stacked = self.counted && area.width < 120;
        let compact_counted = self.counted && area.height < 14;
        let summary_height = if area.height >= 14 {
            6
        } else if self.counted {
            5
        } else {
            4
        };
        let [summary, body, footer] = Layout::vertical([
            Constraint::Length(summary_height),
            Constraint::Min(5),
            Constraint::Length(if compact_counted { 2 } else { 3 }),
        ])
        .areas(area);
        self.page = (usize::from(body.height.saturating_sub(3)) / if stacked { 2 } else { 1 }).max(1);
        self.clamp();
        let Some(reading) = self.reading.as_ref() else {
            frame.render_widget(Paragraph::new("ConTime worker monitor\nWaiting for a complete worker reading"), summary);
            if let Some(error) = &self.error {
                frame.render_widget(Paragraph::new(error.as_str()).style(Style::default().fg(Color::Yellow)), body);
            }
            return;
        };
        let count = reading.workers.len();
        // Widths depend only on the viewport, never on the current values.
        let overview_width = (usize::from(area.width).saturating_sub(6 + " workers |  snapshots |  pending |  due".len()) / 3).clamp(1, 51);
        let title = format!(
            "{} workers | {} snapshots | {} pending | {} due",
            slot(number(count), 6),
            slot(number(self.totals[0]), overview_width),
            slot(number(self.totals[1]), overview_width),
            slot(number(self.totals[2]), overview_width),
        );
        let timings = format!(
            "Age: {} ms | Collection: {} ms",
            slot(format!("{:.3}", now.saturating_sub(reading.completed_at).as_secs_f64() * 1000.0), 12),
            slot(format!("{:.3}", reading.completed_at.saturating_sub(reading.started_at).as_secs_f64() * 1000.0), 12),
        );
        let mut lines = vec![
            Line::styled("ConTime worker monitor", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            clipped_ascii(title, usize::from(area.width)).into(),
            timings.into(),
        ];
        if let Some(activity) = &self.activity {
            let counts = if self.counted {
                let (insert_label, replay_label) =
                    if area.width < 80 { ("Total ins:", "Total replay:") } else { ("Total inserted:", "Total replayed:") };
                let count_width = (usize::from(area.width).saturating_sub(insert_label.len() + replay_label.len() + 5) / 2).clamp(1, 18);
                format!(
                    "{insert_label} {} | {replay_label} {}{}",
                    slot(optional_count(activity.inserted_applications), count_width),
                    slot(optional_count(activity.replay_applications), count_width),
                    if activity.saturated { " | SATURATED (lower bounds)" } else { "" }
                )
            } else {
                "Counters unavailable (enable the counters feature)".into()
            };
            lines.push(clipped_ascii(counts, usize::from(area.width)).into());
            if self.counted {
                let (insert_label, replay_label) =
                    if area.width < 80 { ("Avg ins/s:", "Avg replay/s:") } else { ("Avg inserted/sec:", "Avg replayed/sec:") };
                let average = self.average.as_ref().expect("reading has average");
                let metric_width = (usize::from(area.width).saturating_sub(insert_label.len() + replay_label.len() + 5) / 2).clamp(1, 18);
                lines.push(
                    clipped_ascii(
                        format!(
                            "{insert_label} {} | {replay_label} {} | Event-to-snapshot applications",
                            slot(rate(average.inserted_per_second), metric_width),
                            slot(rate(average.replay_per_second), metric_width)
                        ),
                        usize::from(area.width),
                    )
                    .into(),
                );
            }
        }
        if let Some(error) = &self.error {
            // Preserve host errors even in compact viewports.
            if lines.len() >= summary.height as usize {
                lines.pop();
            }
            lines.push(Line::styled(error.as_str(), Style::default().fg(Color::Yellow)));
        }
        frame.render_widget(Paragraph::new(lines), summary);
        // Keep per-worker throughput visible before optional orchestration gauges.
        let activity_columns = self.counted;
        let activity_totals = self.counted && !stacked;
        let queue_columns = !self.counted || area.width >= 150;
        let detailed = area.width >= if activity_columns { 180 } else { 100 };
        let times = area.width >= if activity_columns { 230 } else { 140 };
        let mut headers = vec!["Worker", "Snapshots"];
        let mut widths = vec![Constraint::Length(8), Constraint::Min(12)];
        if queue_columns {
            headers.extend(["Pending", "Due"]);
            widths.extend([Constraint::Length(12), Constraint::Length(10)]);
        }
        if activity_totals {
            headers.extend(["Total inserted", "Total replay"]);
            widths.extend([Constraint::Length(14); 2]);
        }
        if activity_columns {
            headers.extend(["Avg inserted/sec", "Avg replayed/sec"]);
            widths.extend([Constraint::Length(16); 2]);
        }
        if detailed {
            headers.extend(["Metadata", "Prune", "Round"]);
            widths.extend([Constraint::Length(12), Constraint::Length(8), Constraint::Length(12)]);
        }
        let snapshot_width = if times {
            14
        } else {
            let fixed: usize = widths
                .iter()
                .filter_map(|width| match width {
                    Constraint::Length(value) => Some(usize::from(*value)),
                    _ => None,
                })
                .sum();
            usize::from(area.width).saturating_sub(2 + widths.len() - 1 + fixed)
        };
        widths[1] = Constraint::Length(snapshot_width as u16);
        if times {
            headers.extend(["Target", "Horizon"]);
            // Share additional horizontal space between user-defined timestamps.
            widths[1] = Constraint::Length(14);
            widths.extend([Constraint::Fill(1), Constraint::Fill(1)]);
        }
        let end = self.offset.saturating_add(self.page).min(count);
        let rows = reading.workers[self.offset..end].iter().enumerate().map(|(index, worker)| {
            let mut cells = vec![count_cell(worker.worker_id, 8), count_cell(worker.history_stores, snapshot_width)];
            if queue_columns {
                cells.extend([count_cell(worker.pending_histories, 12), count_cell(worker.due_histories, 10)]);
            }
            if activity_columns {
                let activity = &self.activity.as_ref().expect("reading has activity").workers[self.offset + index];
                if activity_totals {
                    cells.extend([
                        clipped_ascii(optional_count(activity.inserted_applications), 14),
                        clipped_ascii(optional_count(activity.replay_applications), 14),
                    ]);
                }
                let average = &self.average.as_ref().expect("reading has average").workers[self.offset + index];
                cells.extend([clipped_ascii(rate(average.inserted_per_second), 16), clipped_ascii(rate(average.replay_per_second), 16)]);
            }
            if detailed {
                cells.extend([
                    count_cell(worker.metadata_only, 12),
                    count_cell(worker.pending_prune_passes, 8),
                    worker.awaiting_resolution.map_or_else(|| "-".into(), |round| count_cell(round, 12)),
                ]);
            }
            if times {
                cells.extend([worker.processing_target.to_string(), worker.admission_horizon.to_string()]);
            }
            let style = if worker.awaiting_resolution.is_some() { Style::default().fg(Color::Yellow) } else { Style::default() };
            Row::new(cells.into_iter().map(|cell| Cell::from(Line::from(cell).alignment(Alignment::Right)))).style(style)
        });
        if count == 0 {
            frame.render_widget(Paragraph::new("No workers in this reading").block(Block::bordered().title(" Workers ")), body);
        } else if stacked {
            let value_width = usize::from(area.width.saturating_sub(20)) / 2;
            let rows = reading.workers[self.offset..end].iter().enumerate().map(|(index, worker)| {
                let activity = &self.activity.as_ref().expect("reading has activity").workers[self.offset + index];
                let average = &self.average.as_ref().expect("reading has average").workers[self.offset + index];
                let column = |first: String, second: String| {
                    Cell::from(ratatui::text::Text::from(vec![
                        Line::from(first).alignment(Alignment::Right),
                        Line::from(second).alignment(Alignment::Right),
                    ]))
                };
                Row::new(vec![
                    column(count_cell(worker.worker_id, 8), String::new()),
                    column("Total".into(), "Avg/sec".into()),
                    column(
                        clipped_ascii(optional_count(activity.inserted_applications), value_width),
                        clipped_ascii(rate(average.inserted_per_second), value_width),
                    ),
                    column(
                        clipped_ascii(optional_count(activity.replay_applications), value_width),
                        clipped_ascii(rate(average.replay_per_second), value_width),
                    ),
                ])
                .height(2)
            });
            let table = Table::new(rows, [Constraint::Length(8), Constraint::Length(7), Constraint::Fill(1), Constraint::Fill(1)])
                .header(
                    Row::new(["Worker", "Measure", "Inserted", "Replayed"])
                        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                )
                .block(Block::bordered().title(" Workers "));
            frame.render_widget(table, body);
        } else {
            let table = Table::new(rows, widths)
                .header(
                    Row::new(headers.into_iter().map(|header| Cell::from(Line::from(header).alignment(Alignment::Right))))
                        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                )
                .block(Block::bordered().title(" Workers "));
            frame.render_widget(table, body);
        }
        let range =
            if count == 0 { "0 of 0".to_string() } else { format!("{}-{} of {}", number(self.offset + 1), number(end), number(count)) };
        if compact_counted {
            frame.render_widget(
                Paragraph::new(format!(
                    "Workers {range} | Scroll rows/pages; Home/End\nAverages are applications/s; replay includes repeats."
                )),
                footer,
            );
            return;
        }
        frame.render_widget(Paragraph::new(format!(
            "Workers {range} | Scroll rows/pages; Home/End | … = clipped value\nPending/due count snapshots, not events. Due work may be blocked.\nTargets are accepted, not completed. Worker observations are independent.{}",
            if times { "" } else if activity_columns { " Widen to 230 columns for time targets." } else { " Widen to 140 columns for time targets." },
        )), footer);
    }
}
