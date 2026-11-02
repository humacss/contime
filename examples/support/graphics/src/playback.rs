//! Reveal queried arena updates when their delivery animations land.
use crate::report::Report;
use std::{collections::VecDeque, sync::Arc};

pub(crate) const HANDOFF_MS: f64 = 450.0;
#[derive(Default)]
pub(crate) struct Playback {
    clock: f64,
    latest: Option<Arc<Report>>,
    visible: Option<Arc<Report>>,
    pending: VecDeque<(f64, Arc<Report>)>,
}
impl Playback {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// Network drawing stays live; only completed arena query results are delayed.
    pub fn update(&mut self, report: Arc<Report>, elapsed_ms: f64, immediate: bool) -> (Arc<Report>, Arc<Report>) {
        self.clock += elapsed_ms.max(0.0);
        if immediate || self.latest.is_none() {
            self.pending.clear();
            self.visible = Some(Arc::clone(&report));
            self.latest = Some(report);
        } else if self.latest.as_ref().is_some_and(|previous| previous.updates != report.updates) {
            self.pending.push_back((self.clock + HANDOFF_MS, Arc::clone(&report)));
            self.latest = Some(report);
            // Bound memory independently of report frequency. The workload caps
            // queries at 60 Hz, giving at most 28 entries during this interval.
            while self.pending.len() > 64 {
                self.pending.pop_front();
            }
        }
        while self.pending.front().is_some_and(|(due, _)| *due <= self.clock) {
            self.visible = self.pending.pop_front().map(|(_, report)| report);
        }
        (Arc::clone(self.visible.as_ref().unwrap()), Arc::clone(self.latest.as_ref().unwrap()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scene(x: f64, updates: u64) -> Arc<Report> {
        let mut report = crate::animation::tests::scene(x);
        report.updates = updates;
        Arc::new(report)
    }
    #[test]
    fn turn_becomes_visible_at_handoff_completion_not_query_completion() {
        let mut playback = Playback::default();
        playback.update(scene(0.0, 1), 0.0, false);
        let next = scene(10.0, 2);
        let (visible, launch) = playback.update(Arc::clone(&next), 16.0, false);
        assert_eq!(visible.positions[0].position[0], 0.0);
        assert_eq!(launch.positions[0].position[0], 10.0);
        assert_eq!(playback.update(Arc::clone(&next), 449.0, false).0.positions[0].position[0], 0.0);
        assert_eq!(playback.update(next, 1.0, false).0.positions[0].position[0], 10.0);
    }
    #[test]
    fn pauses_freeze_handoffs_and_completion_reset_and_reduced_motion_are_immediate() {
        let mut playback = Playback::default();
        playback.update(scene(0.0, 1), 0.0, false);
        let next = scene(10.0, 2);
        playback.update(Arc::clone(&next), 0.0, false);
        assert_eq!(playback.update(Arc::clone(&next), 0.0, false).0.positions[0].position[0], 0.0);
        assert_eq!(playback.update(next, 0.0, true).0.positions[0].position[0], 10.0);
        assert!(playback.pending.is_empty());
        playback.reset();
        assert!(playback.latest.is_none() && playback.visible.is_none());
        assert_eq!(playback.update(scene(3.0, 1), 0.0, false).0.positions[0].position[0], 3.0);
    }
}
