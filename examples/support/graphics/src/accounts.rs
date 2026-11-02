//! Account presentation only: queried balances, provisional decisions and retry arrivals.
use crate::{
    accounts_report::{Decision, Report},
    labels::{Labels, Style},
};
use bevy::prelude::*;
use std::collections::{BTreeMap, VecDeque};
const TEXT: Color = Color::srgb(0.80, 0.88, 0.92);
const MUTED: Color = Color::srgb(0.48, 0.61, 0.69);
const CREDIT: Color = Color::srgb(0.38, 0.85, 0.73);
const PAYMENT: Color = Color::srgb(0.96, 0.65, 0.48);
const ACCEPT: Color = Color::srgb(0.43, 0.83, 0.67);
const REJECT: Color = Color::srgb(0.96, 0.45, 0.55);
#[derive(Default)]
pub(crate) struct Display {
    visible: Report,
    pending: VecDeque<(f64, Report)>,
    last_revision: Option<u64>,
    clock: f64,
    tracks: BTreeMap<u64, usize>,
}
fn x(time: u64, now: f64) -> f32 {
    (-96.0 + (time as f64 - now + 6500.0) / 10000.0 * 192.0).clamp(-96.0, 96.0) as f32
}
fn mark(gizmos: &mut Gizmos, p: Vec2, amount: i64, color: Color, scale: f32) {
    gizmos.line_2d(p - Vec2::X * scale, p + Vec2::X * scale, color);
    if amount >= 0 {
        gizmos.line_2d(p - Vec2::Y * scale, p + Vec2::Y * scale, color);
    }
}
fn ids(report: &Report) -> Vec<u64> {
    let mut ids: Vec<_> = report.reconciled.iter().chain(&report.arrival_order).map(|d| d.id).collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() > 6 {
        ids.drain(..ids.len() - 6);
    }
    ids
}
impl Display {
    fn observe(&mut self, report: &Report, immediate: bool) {
        if immediate || self.pending.len() >= 64 {
            self.pending.clear();
            self.visible = report.clone();
            self.last_revision = Some(report.revision);
            return;
        }
        if self.last_revision != Some(report.revision) {
            self.pending.push_back((self.clock + 450.0, report.clone()));
            self.last_revision = Some(report.revision);
        }
        while self.pending.front().is_some_and(|(due, _)| *due <= self.clock) {
            self.visible = self.pending.pop_front().unwrap().1;
        }
    }
    pub fn draw(&mut self, report: &Report, elapsed: f64, immediate: bool, labels: &mut Labels, gizmos: &mut Gizmos) {
        self.tracks.retain(|sequence, _| report.deliveries.iter().any(|event| event.sequence == *sequence));
        for event in &report.deliveries {
            let available = |slot| {
                !report.deliveries.iter().any(|other| {
                    other.sequence != event.sequence
                        && self.tracks.get(&other.sequence) == Some(&slot)
                        && event.action_ms <= other.arrived_ms.unwrap_or(other.due_ms).saturating_add(400)
                        && other.action_ms <= event.arrived_ms.unwrap_or(event.due_ms).saturating_add(400)
                })
            };
            if self.tracks.get(&event.sequence).is_some_and(|slot| available(*slot)) {
                continue;
            }
            let slot = (0..128).find(|slot| available(*slot)).expect("bounded timeline preview");
            self.tracks.insert(event.sequence, slot);
        }
        self.clock = (self.clock + elapsed.max(0.0)).max(report.clock_ms as f64).min(report.clock_ms as f64 + 100.0);
        self.observe(report, immediate);
        labels.push(Vec2::new(-100.0, 84.0), 192.0, "TRANSACTION DELIVERY", Style::Heading, TEXT);
        let now_x = x(self.clock as u64, self.clock);
        labels.push(Vec2::new(now_x - 4.0, 78.5), 38.0, "DELIVERY", Style::Legend, TEXT);
        for second in ((self.clock as u64).saturating_sub(6500) / 1000)..=((self.clock as u64 + 3500) / 1000) {
            let tick = x(second * 1000, self.clock);
            gizmos.line_2d(Vec2::new(tick, 49.0), Vec2::new(tick, 75.0), Color::srgba(0.5, 0.65, 0.7, 0.14));
        }
        gizmos.line_2d(Vec2::new(now_x, 49.0), Vec2::new(now_x, 76.0), MUTED);
        for event in &report.deliveries {
            let end = event.arrived_ms.unwrap_or(event.due_ms);
            if end as f64 + 6500.0 < self.clock || event.action_ms as f64 > self.clock + 3500.0 {
                continue;
            }
            let y = 72.0 - self.tracks[&event.sequence] as f32 * 7.0;
            let color = if event.duplicate {
                MUTED
            } else if event.amount >= 0 {
                CREDIT
            } else {
                PAYMENT
            };
            let start = x(event.action_ms, self.clock);
            let end = x(end, self.clock);
            gizmos.line_2d(Vec2::new(start, y), Vec2::new(end, y), color);
            if event.arrived_ms.unwrap_or(event.due_ms) as f64 <= self.clock + 3500.0 {
                gizmos.line_2d(Vec2::new(end, y - 0.7), Vec2::new(end, y + 0.7), color);
            }
            if event.action_ms as f64 + 6500.0 >= self.clock {
                mark(gizmos, Vec2::new(start, y), event.amount, color, 0.8);
                labels.push(
                    Vec2::new(start - 1.0, y + 3.5),
                    35.0,
                    &format!("#{} {:+}{}", event.transaction, event.amount, if event.duplicate { " retry" } else { "" }),
                    Style::Status,
                    color,
                );
            }
            if !immediate && let Some(arrived) = event.arrived_ms {
                let t = ((self.clock - arrived as f64) / 450.0).clamp(0.0, 1.0) as f32;
                if t < 1.0 {
                    let rows = ids(report);
                    if let Some(row) = rows.iter().position(|id| *id == event.transaction) {
                        for offset in [-53.0, 53.0] {
                            let target = Vec2::new(offset, 13.0 - row as f32 * 7.0);
                            mark(
                                gizmos,
                                Vec2::new(x(arrived, self.clock), y).lerp(target, t * t * (3.0 - 2.0 * t)),
                                event.amount,
                                color,
                                1.0,
                            );
                        }
                    }
                }
            }
        }
        labels.push(
            Vec2::new(-100.0, 46.0),
            200.0,
            "+ deposit / refund    − payment request    retry: same transaction ID",
            Style::Legend,
            MUTED,
        );
        let visible = &self.visible;
        let rows = ids(visible);
        for (offset, title, balance, decisions) in [
            (-53.0, "WITH CONTIME", visible.balance, &visible.reconciled),
            (53.0, "ARRIVAL ORDER", visible.arrival_balance, &visible.arrival_order),
        ] {
            labels.push(Vec2::new(offset - 47.0, 35.0), 98.0, title, Style::Heading, TEXT);
            labels.push(Vec2::new(offset - 47.0, 27.0), 95.0, &format!("{balance} credits"), Style::Heading, CREDIT);
            labels.push(Vec2::new(offset - 47.0, 20.0), 98.0, "TRANSACTION    DECISION    BALANCE", Style::Status, MUTED);
            for (row, id) in rows.iter().enumerate() {
                let y = 13.0 - row as f32 * 7.0;
                if let Some(d) = decisions.iter().find(|d| d.id == *id) {
                    let color = if d.accepted { ACCEPT } else { REJECT };
                    labels.push(Vec2::new(offset - 47.0, y), 40.0, &format!("#{id}  {:+}", d.amount), Style::Legend, TEXT);
                    labels.push(Vec2::new(offset - 8.0, y), 34.0, if d.accepted { "Accepted" } else { "Rejected" }, Style::Legend, color);
                    labels.push(Vec2::new(offset + 29.0, y), 21.0, &format!("{}", d.balance), Style::Legend, TEXT);
                }
            }
            graph(decisions, offset, self.clock, gizmos);
        }
        labels.push(
            Vec2::new(-100.0, -48.0),
            200.0,
            &format!("{} duplicate deliveries ignored on both sides", report.retries),
            Style::Caption,
            TEXT,
        );
        labels.push(
            Vec2::new(-100.0, -55.0),
            200.0,
            "Provisional decisions only · No settlement or external transfers · P pauses",
            Style::Legend,
            MUTED,
        );
    }
}
fn graph(decisions: &[Decision], offset: f32, clock: f64, gizmos: &mut Gizmos) {
    let left = offset - 46.0;
    let right = offset + 46.0;
    let baseline = -42.0;
    gizmos.line_2d(Vec2::new(left, baseline), Vec2::new(right, baseline), MUTED);
    let peak = decisions.iter().map(|d| d.balance).max().unwrap_or(100).max(100) as f32;
    let mut previous = Vec2::new(left, baseline);
    for d in decisions {
        let px = (right - (clock - d.time_ms as f64) as f32 / 10000.0 * (right - left)).clamp(left, right);
        let current = Vec2::new(px, baseline + d.balance as f32 / peak * 14.0);
        gizmos.line_2d(previous, Vec2::new(current.x, previous.y), CREDIT);
        gizmos.line_2d(Vec2::new(current.x, previous.y), current, CREDIT);
        previous = current;
    }
    gizmos.line_2d(previous, Vec2::new(right, previous.y), CREDIT);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn same_time_requeries_are_presented_when_their_handoffs_land() {
        let mut display = Display::default();
        let initial = Report { revision: 1, query_ms: 1001, balance: 40, ..Default::default() };
        display.observe(&initial, true);
        let corrected = Report { revision: 2, balance: 20, ..initial };
        display.observe(&corrected, false);
        assert_eq!(display.visible.balance, 40);
        display.clock = 449.0;
        display.observe(&corrected, false);
        assert_eq!(display.visible.balance, 40);
        display.clock = 450.0;
        display.observe(&corrected, false);
        assert_eq!(display.visible.balance, 20);
        assert!(display.pending.is_empty());
    }
}
