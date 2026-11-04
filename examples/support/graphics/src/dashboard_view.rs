//! Shared throughput history and worker diagnostics for the live workloads.
use crate::{
    dashboard::{Dashboard, WorkerRow},
    labels::{Labels, Style},
};
use bevy::prelude::*;
use std::collections::VecDeque;

const HISTORY_SECONDS: f64 = 30.0;
#[derive(Clone, Copy, Debug)]
struct Sample {
    time: f64,
    recent: f64,
    target: f64,
    outstanding: u64,
    memory: Option<u64>,
}
#[derive(Default)]
struct History {
    previous: Option<(f64, u64)>,
    points: VecDeque<Sample>,
}
impl History {
    fn record(&mut self, time: f64, completed: u64, target: u64, outstanding: u64, memory: Option<u64>) {
        let Some((before, count)) = self.previous else {
            self.previous = Some((time, completed));
            return;
        };
        if time < before || completed < count {
            *self = Self::default();
            self.previous = Some((time, completed));
            return;
        }
        let dt = time - before;
        if dt < 0.5 {
            return;
        }
        self.points.push_back(Sample { time, recent: (completed - count) as f64 / dt, target: target as f64, outstanding, memory });
        self.previous = Some((time, completed));
        while self.points.front().is_some_and(|sample| sample.time < time - HISTORY_SECONDS) {
            self.points.pop_front();
        }
        while self.points.len() > 61 {
            self.points.pop_front();
        }
    }
    fn nearest(&self, time: f64) -> Option<&Sample> {
        self.points.iter().min_by(|a, b| (a.time - time).abs().total_cmp(&(b.time - time).abs()))
    }
    fn bounds(&self) -> Option<(f64, f64)> {
        self.points.iter().flat_map(|sample| [sample.recent, sample.target, sample.outstanding as f64]).fold(None, |bounds, value| {
            Some(match bounds {
                None => (value, value),
                Some((min, max)) => (f64::min(min, value), f64::max(max, value)),
            })
        })
    }
    fn memory_bounds(&self) -> Option<(f64, f64)> {
        value_bounds(self.points.iter().filter_map(|sample| sample.memory.map(|value| value as f64)))
    }
}
fn value_bounds(values: impl Iterator<Item = f64>) -> Option<(f64, f64)> {
    values.fold(None, |bounds, value| {
        Some(match bounds {
            None => (value, value),
            Some((min, max)) => (f64::min(min, value), f64::max(max, value)),
        })
    })
}
fn scale_rate(value: f64, min: f64, max: f64) -> f64 {
    if min == max { 0.5 } else { ((value - min) / (max - min)).clamp(0.0, 1.0) }
}
#[derive(Default)]
struct Selection {
    pinned: Option<Sample>,
}
impl Selection {
    fn update(&mut self, hovered: Option<Sample>, clicked: bool, inside: bool) -> Option<Sample> {
        if clicked {
            self.pinned = if inside { hovered } else { None };
        }
        self.pinned.or(hovered)
    }
}
pub(crate) struct Pointer {
    pub position: Option<Vec2>,
    pub clicked: bool,
}
fn plot_left(width: f32) -> f32 {
    -100.0 + (76.0 * 214.0 / width.max(1.0)).min(150.0)
}
fn plot_right(width: f32) -> f32 {
    100.0 - (76.0 * 214.0 / width.max(1.0)).min(150.0)
}
fn worker_totals(rows: &[WorkerRow]) -> Option<WorkerRow> {
    if rows.is_empty() {
        return None;
    }
    let sum = |f: fn(&WorkerRow) -> Option<u64>| rows.iter().try_fold(0u64, |a, row| a.checked_add(f(row)?));
    let saturated = rows.iter().any(|row| row.saturated);
    let rate = if saturated { None } else { rows.iter().try_fold(0.0, |a, row| Some(a + row.inserted_per_second?)) };
    Some(WorkerRow {
        id: 0,
        snapshots: rows.iter().fold(0u64, |a, row| a.saturating_add(row.snapshots)),
        pending_histories: sum(|row| row.pending_histories),
        inserted: sum(|row| row.inserted),
        replayed: None,
        saturated,
        inserted_per_second: rate,
        replayed_per_second: None,
    })
}
fn worker_capacity(worker_top: f32, bottom: f32, stacked: bool) -> usize {
    let (first_offset, row_height, last_offset) = if stacked { (0.0, 20.0, 13.0) } else { (2.0, 6.0, 0.0) };
    (((worker_top - first_offset - last_offset - bottom).max(0.0) / row_height).floor() as usize + 1).clamp(1, 18)
}
#[derive(Default)]
pub(crate) struct View {
    worker_offset: usize,
    history: History,
    selection: Selection,
}
impl View {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn draw(
        &mut self,
        data: &Dashboard,
        keys: &ButtonInput<KeyCode>,
        labels: &mut Labels,
        gizmos: &mut Gizmos,
        size: Vec2,
        pointer: Pointer,
    ) {
        let compact = crate::viewport::dashboard_compact(size);
        let x_scale = crate::viewport::dashboard_horizontal_scale(size);
        let cursor = pointer.position.map(|p| Vec2::new(p.x / x_scale, p.y));
        self.history.record(data.elapsed, data.completed, data.target_rate, data.outstanding, data.resident_bytes);
        let stacked = size.x < 750.0;
        let worker_base = if compact { -1.0 } else { 0.0 };
        let bottom = crate::viewport::dashboard_bottom(size) + if data.failure.is_some() { 9.0 } else { 2.0 };
        let visible = worker_capacity(worker_base, bottom, stacked);
        // Use surplus room for the graph when there are fewer workers.
        let used = data.worker_rows.len().min(visible).max(1);
        let row_height = if stacked { 20.0 } else { 6.0 };
        let graph_extra = (visible - used) as f32 * row_height;
        if keys.just_pressed(KeyCode::ArrowDown) {
            self.worker_offset = self.worker_offset.saturating_add(1);
        }
        if keys.just_pressed(KeyCode::ArrowUp) {
            self.worker_offset = self.worker_offset.saturating_sub(1);
        }
        if keys.just_pressed(KeyCode::PageDown) {
            self.worker_offset = self.worker_offset.saturating_add(visible);
        }
        if keys.just_pressed(KeyCode::PageUp) {
            self.worker_offset = self.worker_offset.saturating_sub(visible);
        }
        self.worker_offset = self.worker_offset.min(data.worker_rows.len().saturating_sub(visible));
        let heading = Color::srgb(0.83, 0.92, 0.95);
        let text = Color::srgb(0.72, 0.81, 0.85);
        let recent_color = Color::srgb(0.45, 0.83, 0.65);
        let average_color = Color::srgb(0.45, 0.83, 0.65);
        let target_color = Color::srgb(0.40, 0.65, 0.98);
        let outstanding_color = Color::srgb(0.96, 0.77, 0.36);
        let memory_color = Color::srgb(0.75, 0.65, 0.96);
        let put = |labels: &mut Labels, x: f32, y, width: f32, value: &str, style, color| {
            labels.push(Vec2::new(x * x_scale, y), width * x_scale, value, style, color)
        };

        // Events share the left scale; memory uses its own right-hand scale.
        let title = if compact { 87. } else { 63. };
        let graph_top = if stacked { title - 23.0 } else { title - 16.0 };
        let graph_bottom = (if compact { 48. } else { 28. }) - graph_extra;
        let (x0, x1) = (plot_left(size.x), plot_right(size.x));
        put(labels, -100., title, 195., "Benchmark history", Style::Heading, heading);
        let memory_bounds = self.history.memory_bounds();
        let memory_legend = if memory_bounds.is_some() { "Memory" } else { "Memory unavailable" };
        if stacked {
            for (x, y, value, color) in [
                (-100., title - 8., "Confirmed", recent_color),
                (0., title - 8., "Target", target_color),
                (-100., title - 15.5, "Outstanding", outstanding_color),
                (0., title - 15.5, memory_legend, memory_color),
            ] {
                put(labels, x, y, 100., value, Style::Status, color);
            }
        } else {
            for (x, value, color) in [
                (-100., "Confirmed", recent_color),
                (-50., "Target", target_color),
                (0., "Outstanding", outstanding_color),
                (50., memory_legend, memory_color),
            ] {
                put(labels, x, title - 8., 50., value, Style::Status, color);
            }
        }
        let rate_bounds = self.history.bounds();
        let grid = Color::srgb(0.23, 0.33, 0.39);
        for y in [graph_bottom, (graph_bottom + graph_top) * 0.5, graph_top] {
            gizmos.line_2d(Vec2::new(x0 * x_scale, y), Vec2::new(x1 * x_scale, y), grid);
        }
        let now = data.elapsed;
        let point = |sample: &Sample, value: f64, bounds: (f64, f64)| {
            Vec2::new(
                (x0 + (((sample.time - (now - HISTORY_SECONDS)) / HISTORY_SECONDS).clamp(0.0, 1.0) as f32) * (x1 - x0)) * x_scale,
                graph_bottom + scale_rate(value, bounds.0, bounds.1) as f32 * (graph_top - graph_bottom),
            )
        };
        for (a, b) in self.history.points.iter().zip(self.history.points.iter().skip(1)) {
            for (av, bv, bounds, color) in [
                (Some(a.recent), Some(b.recent), rate_bounds, recent_color),
                (Some(a.target), Some(b.target), rate_bounds, target_color),
                (Some(a.outstanding as f64), Some(b.outstanding as f64), rate_bounds, outstanding_color),
                (a.memory.map(|n| n as f64), b.memory.map(|n| n as f64), memory_bounds, memory_color),
            ] {
                if let (Some(av), Some(bv), Some(bounds)) = (av, bv, bounds) {
                    gizmos.line_2d(point(a, av, bounds), point(b, bv, bounds), color);
                }
            }
        }
        if let Some((min, max)) = rate_bounds {
            let axis_width = (x0 + 100.0 - 3.0).max(1.0);
            put(labels, -100., graph_top, axis_width, &number(max.round() as u64), Style::Status, text);
            put(labels, -100., graph_bottom, axis_width, &number(min.round() as u64), Style::Status, text);
        } else {
            put(labels, x0, (graph_top + graph_bottom) * 0.5, 100., "Waiting for readings", Style::Status, text);
        }
        if let Some((min, max)) = memory_bounds {
            let right = x1 + 3.0;
            let width = (100.0 - right).max(1.0);
            put(labels, right, graph_top, width, &memory(Some(max as u64)), Style::Status, memory_color);
            put(labels, right, graph_bottom, width, &memory(Some(min as u64)), Style::Status, memory_color);
        }
        let inside = cursor.filter(|p| p.x >= x0 && p.x <= x1 && p.y >= graph_bottom && p.y <= graph_top);
        let hover =
            inside.and_then(|p| self.history.nearest(now - HISTORY_SECONDS + f64::from((p.x - x0) / (x1 - x0)) * HISTORY_SECONDS)).copied();
        let selected = self.selection.update(hover, pointer.clicked, inside.is_some());
        if let Some(sample) = selected {
            if sample.time >= now - HISTORY_SECONDS
                && sample.time <= now
                && let Some(bounds) = rate_bounds
            {
                let p = point(&sample, sample.recent, bounds);
                gizmos.line_2d(Vec2::new(p.x, graph_bottom), Vec2::new(p.x, graph_top), Color::srgb(0.54, 0.63, 0.67));
                for (value, bounds, color) in [
                    (Some(sample.recent), rate_bounds, recent_color),
                    (Some(sample.target), rate_bounds, target_color),
                    (Some(sample.outstanding as f64), rate_bounds, outstanding_color),
                    (sample.memory.map(|n| n as f64), memory_bounds, memory_color),
                ] {
                    if let (Some(value), Some(bounds)) = (value, bounds) {
                        gizmos.circle_2d(point(&sample, value, bounds), 0.8, color).resolution(12);
                    }
                }
            }
            let age = (now - sample.time).max(0.0);
            let timestamp = if self.selection.pinned.is_some() { format!("Pinned {:.0}s", age) } else { format!("{age:.1}s ago") };
            if stacked {
                for (x, y, value, color) in [
                    (-100., graph_bottom - 4., format!("{timestamp} · Confirmed {}", number(sample.recent.round() as u64)), recent_color),
                    (0., graph_bottom - 4., format!("Target {}", number(sample.target.round() as u64)), target_color),
                    (-100., graph_bottom - 10., format!("Outstanding {}", number(sample.outstanding)), outstanding_color),
                    (0., graph_bottom - 10., format!("Memory {}", memory(sample.memory)), memory_color),
                ] {
                    put(labels, x, y, 100., &value, Style::Status, color);
                }
            } else {
                put(labels, -100., graph_bottom - 4., 27., &timestamp, Style::Status, heading);
                for (x, value, color) in [
                    (-73., format!("Confirmed {}", number(sample.recent.round() as u64)), recent_color),
                    (-29., format!("Target {}", number(sample.target.round() as u64)), target_color),
                    (15., format!("Outstanding {}", number(sample.outstanding)), outstanding_color),
                    (59., format!("Memory {}", memory(sample.memory)), memory_color),
                ] {
                    put(labels, x, graph_bottom - 4., 43., &value, Style::Status, color);
                }
            }
        } else {
            put(labels, x0, graph_bottom - 4., 80., "30 seconds ago", Style::Status, text);
            put(labels, x1 - 20., graph_bottom - 4., 20., "Now", Style::Status, text);
        }

        // These event-level figures are distinct from worker history applications.
        let confirmed = rate(self.history.points.back().map(|sample| sample.recent));
        if size.x < 750.0 {
            let top = if compact { 116. } else { 92. };
            for (y, left, right, lc, rc) in [
                (
                    top,
                    format!("Target / sec: {}", number(data.target_rate)),
                    format!("Confirmed / sec: {confirmed}"),
                    target_color,
                    recent_color,
                ),
                (
                    top - 7.,
                    format!("Outstanding: {}", number(data.outstanding)),
                    format!("Memory: {}", memory(data.resident_bytes)),
                    outstanding_color,
                    memory_color,
                ),
            ] {
                put(labels, -100., y, 100., &left, Style::Status, lc);
                put(labels, 3., y, 100., &right, Style::Status, rc);
            }
        } else {
            let top = if compact { 116. } else { 92. };
            for (x, title, value, color) in [
                (-100., "Target events / sec", number(data.target_rate), target_color),
                (-50., "Confirmed events / sec", confirmed, recent_color),
                (0., "Outstanding events", number(data.outstanding), outstanding_color),
                (50., "Workload memory", memory(data.resident_bytes), memory_color),
            ] {
                put(labels, x, top, 48., title, Style::Status, text);
                put(labels, x, top - 7., 48., &value, Style::Caption, color);
            }
        }
        let table_top = (if compact { 30. } else { 19. }) - graph_extra;
        let start = if data.worker_rows.is_empty() { 0 } else { self.worker_offset + 1 };
        let end = (self.worker_offset + visible).min(data.worker_rows.len());
        let title = if size.x < 640.0 {
            format!("History applications  ·  {start}–{end} of {}  ·  ↑ / ↓", number(data.workers as u64))
        } else {
            format!("History applications  ·  Workers {start}–{end} of {}  ·  ↑ / ↓  or  Page Up / Down", number(data.workers as u64))
        };
        put(labels, -100., table_top, 195., &title, Style::Caption, heading);
        let total = (data.worker_rows.len() == data.workers).then(|| worker_totals(&data.worker_rows)).flatten();
        if !stacked {
            for (x, width, name) in [
                (-100., 18., "Worker"),
                (-82., 23., "Histories"),
                (-59., 36., "Inserted / sec"),
                (-23., 36., "Inserted total"),
                (13., 35., "Pending"),
                (48., 52., "Verified"),
            ] {
                put(labels, x, table_top - 7., width, name, Style::Status, text);
            }
            for (x, width, value, color) in [
                (-100., 18., "Total".to_owned(), heading),
                (-82., 23., total.as_ref().map_or_else(|| "–".into(), |v| number(v.snapshots)), heading),
                (-59., 36., rate(total.as_ref().and_then(|v| v.inserted_per_second)), average_color),
                (-23., 36., total.as_ref().map_or_else(|| "–".into(), |v| count(v.inserted, v.saturated)), heading),
                (13., 35., total.as_ref().map_or_else(|| "–".into(), |v| count(v.pending_histories, false)), heading),
                (
                    48.,
                    52.,
                    data.hash_summary.as_ref().map_or_else(|| "–".into(), |v| verified(v.up_to_date, data.snapshots as u64)),
                    heading,
                ),
            ] {
                put(labels, x, table_top - 14., width, &value, Style::Status, color);
            }
        } else {
            put(
                labels,
                -100.,
                table_top - 7.,
                195.,
                &format!("Total  ·  {} histories", total.as_ref().map_or_else(|| "–".into(), |v| number(v.snapshots))),
                Style::Status,
                heading,
            );
            let inserted = total.as_ref().map_or_else(|| "–".into(), |v| count(v.inserted, v.saturated));
            let pending = total.as_ref().map_or_else(|| "–".into(), |v| count(v.pending_histories, false));
            put(
                labels,
                -100.,
                table_top - 13.5,
                100.,
                &format!("Inserted / sec: {}", rate(total.as_ref().and_then(|v| v.inserted_per_second))),
                Style::Status,
                average_color,
            );
            put(labels, 3., table_top - 13.5, 100., &format!("Inserted: {inserted}"), Style::Status, heading);
            put(labels, -100., table_top - 20., 100., &format!("Pending histories: {pending}"), Style::Status, heading);
            let verification = data.hash_summary.as_ref().map_or_else(|| "–".into(), |v| verified(v.up_to_date, data.snapshots as u64));
            put(labels, 3., table_top - 20., 100., &format!("Verified: {verification}"), Style::Status, heading);
        }
        let worker_top = worker_base - graph_extra;
        for (index, worker) in data.worker_rows.iter().skip(self.worker_offset).take(visible).enumerate() {
            let y = worker_top - if stacked { index as f32 * 20. } else { 2. + index as f32 * 6. };
            if stacked {
                let title = format!("Worker {}  ·  {} histories", number(worker.id), number(worker.snapshots));
                put(labels, -100., y, 195., &title, Style::Status, heading);
                for (x, offset, value, color) in [
                    (-100., 6.5, format!("Inserted / sec: {}", rate(worker.inserted_per_second)), average_color),
                    (3., 6.5, format!("Inserted: {}", count(worker.inserted, worker.saturated)), text),
                    (-100., 13., format!("Pending histories: {}", count(worker.pending_histories, false)), text),
                    (3., 13., format!("Verified: {}", worker_verified(data, worker.id)), text),
                ] {
                    put(labels, x, y - offset, 100., &value, Style::Status, color);
                }
            } else {
                for (x, width, value, color) in [
                    (-100., 18., number(worker.id), text),
                    (-82., 23., number(worker.snapshots), text),
                    (-59., 36., rate(worker.inserted_per_second), average_color),
                    (-23., 36., count(worker.inserted, worker.saturated), text),
                    (13., 35., count(worker.pending_histories, false), text),
                    (48., 52., worker_verified(data, worker.id), text),
                ] {
                    put(labels, x, y, width, &value, Style::Status, color);
                }
            }
        }
        if let Some(failure) = &data.failure {
            put(
                labels,
                -100.,
                bottom - 4.,
                195.,
                &format!("Verification stopped: {failure}"),
                Style::Caption,
                Color::srgb(0.98, 0.48, 0.48),
            );
        }
    }
}

fn verified(matched: u64, total: u64) -> String {
    if total == 0 {
        return "0 / 0 (–)".into();
    }
    format!("{} / {} ({:.0}%)", number(matched), number(total), matched as f64 * 100.0 / total as f64)
}
fn worker_verified(data: &Dashboard, id: u64) -> String {
    data.hash_summary
        .as_ref()
        .and_then(|summary| summary.workers.as_ref())
        .and_then(|workers| usize::try_from(id).ok().and_then(|id| workers.get(id)))
        .and_then(|worker| Some(verified(worker.up_to_date, worker.total()?)))
        .unwrap_or_else(|| "–".into())
}
fn memory(bytes: Option<u64>) -> String {
    match bytes {
        None => "Unavailable".into(),
        Some(bytes) if bytes >= 1024 * 1024 * 1024 => format!("{:.2} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0)),
        Some(bytes) => format!("{} MiB", number((bytes as f64 / (1024.0 * 1024.0)).round() as u64)),
    }
}
fn number(value: u64) -> String {
    let s = value.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
fn rate(value: Option<f64>) -> String {
    value.filter(|v| v.is_finite() && *v >= 0.0).map_or_else(|| "–".into(), |v| number(v.round() as u64))
}
fn count(value: Option<u64>, saturated: bool) -> String {
    value.map_or_else(|| "–".into(), |n| format!("{}{}", if saturated { "≥" } else { "" }, number(n)))
}

#[cfg(test)]
mod history_tests {
    use super::*;
    #[test]
    fn samples_distinct_rates_and_selects_the_nearest_observed_time() {
        let mut history = History::default();
        history.record(1.0, 100, 500, 0, None);
        history.record(1.25, 130, 600, 0, None);
        assert!(history.points.is_empty());
        history.record(2.0, 300, 700, 0, None);
        history.record(3.0, 400, 800, 0, None);
        let sample = history.nearest(2.1).unwrap();
        assert_eq!((sample.time, sample.recent, sample.target), (2.0, 200.0, 700.0));
        assert_eq!(history.nearest(2.9).unwrap().time, 3.0);
        history.record(0.0, 0, 0, 0, None);
        assert!(history.nearest(2.0).is_none());
    }
    #[test]
    fn chart_bounds_cover_only_visible_samples_and_handle_flat_rates() {
        let mut history = History::default();
        assert_eq!(history.bounds(), None);
        history.record(1.0, 100, 500, 0, None);
        history.record(2.0, 300, 700, 0, None);
        history.record(3.0, 400, 800, 0, None);
        assert_eq!(history.bounds(), Some((0.0, 800.0)));
        assert_eq!(scale_rate(100.0, 100.0, 800.0), 0.0);
        assert_eq!(scale_rate(800.0, 100.0, 800.0), 1.0);
        assert_eq!(scale_rate(500.0, 500.0, 500.0), 0.5);
        history.record(40.0, 400, 300, 0, None);
        let (min, max) = history.bounds().unwrap();
        assert_eq!(min, 0.0);
        assert_eq!(max, 300.0);
    }
    #[test]
    fn event_scale_includes_outstanding_counts_at_both_extremes() {
        let mut history = History::default();
        history.record(1.0, 0, 100, 0, None);
        history.record(2.0, 100, 100, 2, Some(1024));
        history.record(3.0, 200, 100, 250, Some(2048));
        assert_eq!(history.bounds(), Some((2.0, 250.0)));
        assert_eq!(history.memory_bounds(), Some((1024.0, 2048.0)));
    }
    #[test]
    fn shared_event_scale_keeps_missing_memory_distinct_from_zero() {
        let mut history = History::default();
        history.record(1.0, 100, 500, 5, None);
        history.record(2.0, 300, 700, 12, Some(1000));
        history.record(3.0, 400, 800, 3, None);
        history.record(4.0, 600, 700, 9, Some(2000));
        assert_eq!(history.bounds(), Some((3.0, 800.0)));
        assert_eq!(history.memory_bounds(), Some((1000.0, 2000.0)));
        assert_eq!(history.points[1].memory, None);
        assert_eq!(history.points[1].outstanding, 3);
    }
    #[test]
    fn click_pins_a_sample_until_replaced_or_released() {
        let first = Sample { time: 1.0, recent: 20.0, target: 30.0, outstanding: 2, memory: None };
        let later = Sample { time: 2.0, recent: 40.0, target: 50.0, outstanding: 3, memory: Some(1024) };
        let mut selection = Selection::default();
        assert_eq!(selection.update(Some(first), false, true).unwrap().time, 1.0);
        assert_eq!(selection.update(Some(first), true, true).unwrap().time, 1.0);
        assert_eq!(selection.update(Some(later), false, true).unwrap().time, 1.0);
        assert_eq!(selection.update(None, false, false).unwrap().time, 1.0);
        assert_eq!(selection.update(Some(later), true, true).unwrap().time, 2.0);
        assert!(selection.update(None, true, false).is_none());
    }
    #[test]
    fn scale_labels_have_a_separate_fixed_width_gutter() {
        for width in [390.0, 1200.0, 1920.0] {
            let gutter_pixels = (plot_left(width) + 100.0) * width / 214.0;
            assert!((gutter_pixels - 76.0).abs() < 0.01);
            assert!(plot_left(width) < 95.0);
        }
    }
    #[test]
    fn combined_graph_reserves_gutters_and_scales_series_independently() {
        for width in [390.0, 1200.0, 1920.0] {
            assert!(plot_left(width) < plot_right(width));
            assert!(((100.0 - plot_right(width)) * width / 214.0 - 76.0).abs() < 0.01);
        }
        assert_eq!(scale_rate(5.0, 0.0, 10.0), 0.5);
        assert_eq!(scale_rate(2000.0, 1000.0, 3000.0), 0.5);
    }
    #[test]
    fn worker_capacity_uses_available_height_and_respects_label_budget() {
        let normal = worker_capacity(-17.0, -112.0, false);
        assert!(normal > 7);
        assert!(normal <= 18);
        assert!(-17.0 - 2.0 - (normal - 1) as f32 * 6.0 >= -112.0);
        let narrow = worker_capacity(-16.0, -112.0, true);
        assert!(narrow > 1 && narrow < normal);
        assert!(-16.0 - (narrow - 1) as f32 * 20.0 - 13.0 >= -112.0);
    }
    #[test]
    fn worker_totals_only_use_complete_measurements() {
        use crate::dashboard::WorkerRow;
        let row = |id, inserted, pending, rate| WorkerRow {
            id,
            snapshots: 2,
            pending_histories: pending,
            inserted,
            replayed: None,
            saturated: false,
            inserted_per_second: rate,
            replayed_per_second: None,
        };
        let rows = vec![row(0, Some(8), Some(1), Some(4.0)), row(1, Some(5), Some(2), Some(3.0))];
        let totals = worker_totals(&rows).unwrap();
        assert_eq!(
            (totals.snapshots, totals.inserted, totals.pending_histories, totals.inserted_per_second),
            (4, Some(13), Some(3), Some(7.0))
        );
        let missing = vec![rows[0].clone(), row(1, None, None, None)];
        let totals = worker_totals(&missing).unwrap();
        assert_eq!((totals.inserted, totals.pending_histories, totals.inserted_per_second), (None, None, None));
        assert!(worker_totals(&[]).is_none());
    }
    #[test]
    fn verification_format_handles_empty_and_fractional_totals() {
        assert_eq!(verified(0, 0), "0 / 0 (–)");
        assert_eq!(verified(28, 32), "28 / 32 (88%)");
        assert_eq!(verified(1000, 1000), "1,000 / 1,000 (100%)");
    }
    #[test]
    fn formats_rates_with_separators() {
        assert_eq!(rate(Some(1234567.0)), "1,234,567");
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;
    #[test]
    fn memory_uses_readable_units_and_marks_unavailable_values() {
        assert_eq!(memory(None), "Unavailable");
        assert_eq!(memory(Some(1536 * 1024 * 1024)), "1.50 GiB");
        assert_eq!(memory(Some(42 * 1024 * 1024)), "42 MiB");
    }
}
