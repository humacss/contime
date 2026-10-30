//! Stable tracks within each client's timeline row. Only presentation uses them.
use crate::report::{NetworkEvent, Report};
use std::collections::BTreeMap;

// Prefer three well-spaced tracks. Dense previews fill the gaps without leaving
// the client's row. Additional dense tracks remain bounded by the report limit.
const OFFSETS: [f32; 8] = [0.0, 1.4, -1.4, 0.7, -0.7, 2.1, -2.1, 0.35];
#[derive(Default)]
pub(crate) struct Tracks(BTreeMap<u64, usize>);
fn overlaps(a: &NetworkEvent, b: &NetworkEvent) -> bool {
    // Include space for direction icons and delivery tips, not just delay lines.
    let end = |e: &NetworkEvent| e.arrived_ns.unwrap_or(e.delivery_ns).saturating_add(150_000_000);
    a.client == b.client && a.action_ns <= end(b) && b.action_ns <= end(a)
}
impl Tracks {
    pub fn reset(&mut self) {
        self.0.clear();
    }
    pub fn update(&mut self, report: &Report) {
        self.0.retain(|id, _| report.network.iter().any(|e| e.id == *id));
        let mut inputs: Vec<_> = report.network.iter().collect();
        inputs.sort_by_key(|e| (e.action_ns, e.id));
        for event in inputs {
            // A query can postpone handoff beyond the scheduled time. Keep a
            // track only while the current intervals still fit without overlap.
            let available = |slot: &usize| {
                !report.network.iter().any(|other| other.id != event.id && overlaps(event, other) && self.0.get(&other.id) == Some(slot))
            };
            if self.0.get(&event.id).is_some_and(available) {
                continue;
            }
            let slot =
                (0..crate::report::MAX_NETWORK_EVENTS).find(available).expect("bounded preview inputs fit the same number of tracks");
            self.0.insert(event.id, slot);
        }
    }
    pub fn offset(&self, id: u64) -> f32 {
        let slot = self.0.get(&id).copied().unwrap_or(0);
        if slot < OFFSETS.len() {
            OFFSETS[slot]
        } else {
            // Interior points avoid the existing track fractions and row boundaries.
            -2.1 + 4.2 * (slot - 7) as f32 / (crate::report::MAX_NETWORK_EVENTS - 7) as f32
        }
    }
    pub fn icon_scale(&self, id: u64) -> f32 {
        match self.0.get(&id).copied().unwrap_or(0) {
            0..=2 => 1.05,
            3..=7 => 0.3,
            _ => 0.03,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn input(id: u64, client: usize, start: u64, end: u64) -> NetworkEvent {
        NetworkEvent { id, client, direction: [1.0, 0.0], action_ns: start, delivery_ns: end, arrived_ns: None, reordered: false }
    }
    #[test]
    fn overlapping_ranges_get_stable_tracks_and_disjoint_ranges_reuse_them() {
        let mut report = crate::animation::tests::scene(0.0);
        report.network = vec![
            input(1, 2, 0, 1_200_000_000),
            input(2, 2, 1_000_000_000, 2_000_000_000),
            input(3, 2, 3_000_000_000, 4_000_000_000),
            input(4, 3, 0, 1_200_000_000),
        ];
        let mut tracks = Tracks::default();
        tracks.update(&report);
        assert_ne!(tracks.offset(1), tracks.offset(2));
        assert_eq!(tracks.offset(1), tracks.offset(3));
        assert_eq!(tracks.offset(1), tracks.offset(4));
        let second = tracks.offset(2);
        report.network.reverse();
        tracks.update(&report);
        assert_eq!(tracks.offset(2), second);
        report.network.retain(|e| e.id != 1);
        report.network.iter_mut().find(|e| e.id == 2).unwrap().arrived_ns = Some(2_000_000_001);
        tracks.update(&report);
        assert_eq!(tracks.offset(2), second, "delivery and expiration must not move an input");
        tracks.reset();
        assert!(tracks.0.is_empty());
    }
    #[test]
    fn overdue_handoffs_recheck_shared_tracks_without_moving_unrelated_inputs() {
        let mut report = crate::animation::tests::scene(0.0);
        report.network = vec![
            input(1, 1, 0, 100_000_000),
            input(2, 1, 300_000_000, 400_000_000),
            input(3, 1, 900_000_000, 1_000_000_000),
            input(4, 2, 0, 100_000_000),
        ];
        let mut tracks = Tracks::default();
        tracks.update(&report);
        assert_eq!(tracks.offset(1), tracks.offset(2));
        let unrelated = [tracks.offset(3), tracks.offset(4)];
        for event in &mut report.network[..2] {
            event.arrived_ns = Some(500_000_000);
        }
        tracks.update(&report);
        assert_ne!(tracks.offset(1), tracks.offset(2), "actual delivery extended the intervals into an overlap");
        assert_eq!([tracks.offset(3), tracks.offset(4)], unrelated);
        let resolved = [tracks.offset(1), tracks.offset(2)];
        report.network.reverse();
        tracks.update(&report);
        assert_eq!([tracks.offset(1), tracks.offset(2)], resolved, "resolved tracks must remain stable");
    }
    #[test]
    fn dense_preview_stays_in_one_client_band() {
        let mut report = crate::animation::tests::scene(0.0);
        report.network = (1..=crate::report::MAX_NETWORK_EVENTS as u64).map(|id| input(id, 1, 0, 1_000_000_000)).collect();
        let mut tracks = Tracks::default();
        tracks.update(&report);
        for a in &report.network {
            assert!(tracks.offset(a.id).abs() + 0.55 * tracks.icon_scale(a.id) * 1.9 < 2.5);
            for b in &report.network {
                if a.id != b.id {
                    assert_ne!(tracks.offset(a.id), tracks.offset(b.id));
                }
            }
        }
        report.network.clear();
        tracks.update(&report);
        assert!(tracks.0.is_empty());
    }
}
