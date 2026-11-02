//! Stable input tracks within each service row. Shared by lines and handoffs.
use super::{Input, Report};
use std::collections::BTreeMap;
const OFFSETS: [f32; 8] = [0.0, 0.24, -0.24, 0.12, -0.12, 0.30, -0.30, 0.06];
#[derive(Default)]
pub(super) struct Tracks(BTreeMap<u64, usize>);
fn overlaps(a: &Input, b: &Input) -> bool {
    // Reserve room for symbols too, including zero-delay inputs.
    let end = |input: &Input| input.arrived_ms.unwrap_or(input.delivery_ms).saturating_add(150);
    a.service == b.service && a.original_ms <= end(b) && b.original_ms <= end(a)
}
impl Tracks {
    pub(super) fn update(&mut self, report: &Report) {
        self.0.retain(|id, _| report.inputs.iter().any(|input| input.id == *id));
        let mut order: Vec<_> = report.inputs.iter().collect();
        order.sort_by_key(|input| (input.original_ms, input.id));
        for input in order {
            let available = |slot: &usize| {
                !report.inputs.iter().any(|other| other.id != input.id && overlaps(input, other) && self.0.get(&other.id) == Some(slot))
            };
            if self.0.get(&input.id).is_some_and(available) {
                continue;
            }
            let slot = (0..crate::uptime_report::MAX_INPUTS).find(available).expect("bounded input preview fits its tracks");
            self.0.insert(input.id, slot);
        }
    }
    pub(super) fn offset(&self, id: u64) -> f32 {
        let slot = self.0.get(&id).copied().unwrap_or(0);
        if slot < OFFSETS.len() { OFFSETS[slot] } else { -0.30 + 0.60 * (slot - 7) as f32 / (crate::uptime_report::MAX_INPUTS - 7) as f32 }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::uptime_report::Status;
    fn input(id: u64, service: usize, start: u64, end: u64) -> Input {
        Input { id, service, status: Status::Down, original_ms: start, delivery_ms: end, arrived_ms: None, late: false }
    }
    #[test]
    fn overlapping_inputs_separate_and_expiration_does_not_shift_survivors() {
        let mut report = Report {
            inputs: vec![input(1, 1, 100, 1500), input(2, 1, 1000, 2000), input(3, 1, 3000, 4000), input(4, 2, 100, 1500)],
            ..Default::default()
        };
        let mut tracks = Tracks::default();
        tracks.update(&report);
        assert_ne!(tracks.offset(1), tracks.offset(2));
        assert_eq!(tracks.offset(1), tracks.offset(3));
        assert_eq!(tracks.offset(1), tracks.offset(4));
        let position = tracks.offset(2);
        report.inputs.reverse();
        tracks.update(&report);
        assert_eq!(tracks.offset(2), position);
        report.inputs.retain(|input| input.id != 1);
        report.inputs.iter_mut().find(|input| input.id == 2).unwrap().arrived_ms = Some(2100);
        tracks.update(&report);
        assert_eq!(tracks.offset(2), position);
    }
    #[test]
    fn symbols_get_clearance_even_without_delay_and_new_tracks_do_not_rescale_old_ones() {
        let mut report = Report { inputs: vec![input(1, 1, 1000, 1000)], ..Default::default() };
        let mut tracks = Tracks::default();
        tracks.update(&report);
        let first = tracks.offset(1);
        report.inputs.push(input(2, 1, 1100, 1400));
        tracks.update(&report);
        assert_eq!(tracks.offset(1), first);
        assert_ne!(tracks.offset(1), tracks.offset(2));
        for id in 3..=128 {
            report.inputs.push(input(id, 1, 1000, 2000));
        }
        tracks.update(&report);
        assert_eq!(tracks.offset(1), first);
        for input in &report.inputs {
            assert!(tracks.offset(input.id).abs() <= 0.30);
        }
    }
}
