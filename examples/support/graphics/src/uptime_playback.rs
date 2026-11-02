//! Present queried service histories when their paired delivery symbols land.
//! Only presentation is buffered; workload processing and the network stay live.
use super::{NetworkLayout, Report, Tracks, Transfers};
use std::{collections::VecDeque, sync::Arc};
#[derive(Default)]
pub(crate) struct Playback {
    clock: f64,
    visible_due: f64,
    latest: Option<Arc<Report>>,
    visible: Option<Arc<Report>>,
    pending: VecDeque<(f64, Arc<Report>)>,
    transfers: Transfers,
    tracks: Tracks,
    layout: Option<NetworkLayout>,
}
impl Playback {
    pub(crate) fn update(&mut self, report: Arc<Report>, elapsed_ms: f64, immediate: bool) -> Arc<Report> {
        self.clock += elapsed_ms.max(0.0);
        let initialize = self.latest.is_none();
        // At the supported 60 Hz query limit there are at most 28 pending frames.
        // If a custom source exceeds the bound, synchronize without animations.
        let immediate = immediate || initialize || self.pending.len() >= 64;
        self.tracks.update(&report);
        self.layout = Some(NetworkLayout::with_tracks(&report, &self.tracks));
        self.transfers.update(&report, self.layout.as_ref().unwrap(), elapsed_ms, immediate);
        if immediate {
            self.pending.clear();
            self.visible_due = self.clock;
            self.visible = Some(Arc::clone(&report));
            self.latest = Some(report);
        } else if self.latest.as_ref().is_some_and(|previous| previous.updates != report.updates || previous.now_ms != report.now_ms) {
            self.pending.push_back((self.clock + super::transfers::DURATION_MS, Arc::clone(&report)));
            self.latest = Some(report);
        }
        while self.pending.front().is_some_and(|(due, _)| *due <= self.clock) {
            let (due, report) = self.pending.pop_front().unwrap();
            self.visible_due = due;
            self.visible = Some(report);
        }
        Arc::clone(self.visible.as_ref().unwrap())
    }
    pub(crate) fn history_ms(&self) -> u64 {
        let current = self.visible.as_ref().expect("update before drawing").now_ms;
        let Some((due, next)) = self.pending.front() else { return current };
        // Future query timestamps are already buffered for the delivery handoff.
        // Interpolate viewport motion every render frame, never status or totals.
        let fraction = ((self.clock - self.visible_due) / (due - self.visible_due).max(1.0)).clamp(0.0, 1.0);
        (current as f64 + next.now_ms.saturating_sub(current) as f64 * fraction).round() as u64
    }
    pub(crate) fn layout(&self) -> &NetworkLayout {
        self.layout.as_ref().expect("update before drawing")
    }
    pub(crate) fn draw(&self, gizmos: &mut bevy::prelude::Gizmos) {
        self.transfers.draw(gizmos);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::uptime_report::{Input, Interval, ServiceReport, Status, Timeline};
    fn report(updates: u64, status: Status, original_ms: u64, arrived_ms: u64) -> Arc<Report> {
        let timeline =
            Timeline { status, intervals: vec![Interval { start_ms: original_ms, end_ms: 1_000, status }], ..Default::default() };
        Arc::new(Report {
            clock_ms: 1_000,
            now_ms: 1_000,
            updates,
            services: vec![ServiceReport { id: 1, name: "Service 1".into(), reconciled: timeline.clone(), arrival_order: timeline }],
            inputs: if updates == 0 {
                vec![]
            } else {
                vec![Input {
                    id: updates,
                    service: 1,
                    status,
                    original_ms,
                    delivery_ms: arrived_ms,
                    arrived_ms: Some(arrived_ms),
                    late: true,
                }]
            },
            ..Default::default()
        })
    }
    #[test]
    fn history_scrolls_each_render_frame_but_status_waits_for_landing() {
        let mut playback = Playback::default();
        playback.update(report(0, Status::Healthy, 0, 0), 0.0, false);
        let mut outage = report(1, Status::Down, 200, 800);
        Arc::make_mut(&mut outage).now_ms = 1050;
        playback.update(outage, 50.0, false);
        let mut recovery = report(2, Status::Healthy, 400, 900);
        Arc::make_mut(&mut recovery).now_ms = 1100;
        playback.update(Arc::clone(&recovery), 50.0, false);
        assert_eq!(playback.update(Arc::clone(&recovery), 400.0, false).updates, 1);
        assert_eq!(playback.history_ms(), 1050);
        let visible = playback.update(Arc::clone(&recovery), 16.0, false);
        assert_eq!(playback.history_ms(), 1066);
        assert_eq!(visible.services[0].reconciled.status, Status::Down);
        playback.update(Arc::clone(&recovery), 0.0, false);
        assert_eq!(playback.history_ms(), 1066, "pause freezes viewport motion");
        playback.update(Arc::clone(&recovery), 33.0, false);
        assert_eq!(playback.history_ms(), 1099);
        assert_eq!(playback.update(recovery, 1.0, false).updates, 2);
        assert_eq!(playback.history_ms(), 1100);
    }
    #[test]
    fn late_status_and_history_symbols_appear_together_when_both_flights_land() {
        let mut playback = Playback::default();
        playback.update(report(0, Status::Healthy, 0, 0), 0.0, false);
        let outage = report(1, Status::Down, 200, 800);
        let visible = playback.update(Arc::clone(&outage), 0.0, false);
        assert_eq!(visible.services[0].reconciled.status, Status::Healthy);
        assert!(visible.inputs.is_empty());
        assert_eq!(playback.transfers.flight_count(), 2);
        assert_eq!(playback.update(Arc::clone(&outage), 449.0, false).updates, 0);
        let visible = playback.update(outage, 1.0, false);
        assert_eq!(visible.services[0].reconciled.status, Status::Down);
        assert_eq!(visible.services[0].arrival_order.status, Status::Down);
        assert_eq!(visible.inputs[0].original_ms, 200);
        assert_eq!(visible.inputs[0].arrived_ms, Some(800));
        assert_eq!(playback.transfers.flight_count(), 0);
    }
    #[test]
    fn overlapping_reports_land_in_order_and_pause_freezes_both_histories() {
        let mut playback = Playback::default();
        playback.update(report(0, Status::Healthy, 0, 0), 0.0, false);
        let outage = report(1, Status::Down, 200, 800);
        let recovery = report(2, Status::Healthy, 400, 900);
        playback.update(outage, 0.0, false);
        assert_eq!(playback.update(Arc::clone(&recovery), 100.0, false).updates, 0);
        assert_eq!(playback.update(Arc::clone(&recovery), 0.0, false).updates, 0);
        assert_eq!(playback.update(Arc::clone(&recovery), 350.0, false).updates, 1);
        assert_eq!(playback.update(recovery, 100.0, false).updates, 2);
        assert_eq!(playback.transfers.flight_count(), 0);
    }
    #[test]
    fn reduced_motion_finish_and_reset_have_no_pending_animation() {
        let mut playback = Playback::default();
        playback.update(report(0, Status::Healthy, 0, 0), 0.0, false);
        let outage = report(1, Status::Down, 200, 800);
        playback.update(Arc::clone(&outage), 0.0, false);
        assert_eq!(playback.update(outage, 0.0, true).updates, 1);
        assert!(playback.pending.is_empty());
        assert_eq!(playback.transfers.flight_count(), 0);
        playback = Playback::default();
        assert_eq!(playback.update(report(0, Status::Healthy, 0, 0), 0.0, false).updates, 0);
    }
}
