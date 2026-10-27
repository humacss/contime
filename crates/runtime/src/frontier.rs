use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use contime_progress::{CoverageConstraint, MeasurementRound};

pub(crate) type AdmissionObservation<T> = Arc<Mutex<CoverageConstraint<T, u128>>>;

struct ObservationRound<T> {
    measurement: MeasurementRound<T>,
    admissions: Vec<AdmissionObservation<T>>,
}

/// Coordinator-owned conservative boundary. A round accounts for work admitted
/// during measurement as well as each worker's earliest possible publication.
pub(crate) struct Frontier<T> {
    safe: T,
    requested: T,
    workers: NonZeroUsize,
    sequence: u64,
    round: Option<MeasurementRound<T>>,
    observation: Option<ObservationRound<T>>,
    round_prune_limit: T,
}

impl<T: Clone + Default + Ord> Frontier<T> {
    pub(crate) fn new(workers: usize) -> Self {
        let workers = NonZeroUsize::new(workers).expect("at least one worker");
        Self {
            safe: T::default(),
            requested: T::default(),
            workers,
            sequence: 0,
            round: None,
            observation: None,
            round_prune_limit: T::default(),
        }
    }

    pub(crate) fn safe(&self) -> &T {
        &self.safe
    }
    pub(crate) fn requested(&self) -> &T {
        &self.requested
    }
    pub(crate) fn measuring(&self) -> bool {
        self.round.is_some()
    }

    pub(crate) fn request(&mut self, cutoff: T) {
        if cutoff > self.requested {
            self.requested = cutoff;
        }
    }

    pub(crate) fn begin(&mut self) -> Option<u64> {
        self.begin_observing(None)
    }

    pub(crate) fn begin_observing(&mut self, observation_limit: Option<T>) -> Option<u64> {
        if self.round.is_some() || (observation_limit.is_none() && self.requested <= self.safe) {
            return None;
        }
        self.sequence = self.sequence.checked_add(1).expect("measurement round overflow");
        self.round_prune_limit = self.requested.clone();
        let limit = observation_limit.as_ref().map_or_else(|| self.requested.clone(), |limit| limit.clone().max(self.requested.clone()));
        self.round = Some(MeasurementRound::new(self.sequence, limit, self.workers));
        self.observation = observation_limit.map(|limit| ObservationRound {
            measurement: MeasurementRound::new(self.sequence, limit, self.workers),
            admissions: Vec::new(),
        });
        Some(self.sequence)
    }

    #[cfg(test)]
    pub(crate) fn admitted(&mut self, _time: &T) {
        self.admitted_to(|| vec![u128::MAX]);
    }

    pub(crate) fn admitted_to(&mut self, routes: impl FnOnce() -> Vec<u128>) -> Option<AdmissionObservation<T>> {
        // Keep the original pruning constraint, regardless of route coverage.
        if let Some(round) = &mut self.round {
            round.constrain(self.safe.clone());
        }
        let observation = self.observation.as_mut()?;
        let admission = Arc::new(Mutex::new(CoverageConstraint::new(self.safe.clone(), routes())));
        observation.admissions.push(admission.clone());
        Some(admission)
    }

    pub(crate) fn report(&mut self, id: u64, worker: usize, minimum: Option<T>) -> Option<T> {
        let completed = self.round.as_mut()?.report(id, worker, minimum.clone());
        let observed = self.observation.as_mut().and_then(|observation| observation.measurement.report(id, worker, minimum));
        let completed = completed?;
        // Freeze admission bounds on the last report. Until then a worker may
        // replace an uncovered route's fallback with its actual replay bound.
        let observed = observed.map(|mut before| {
            for admission in &self.observation.as_ref().unwrap().admissions {
                if let Some(boundary) = admission.lock().expect("admission observation poisoned").boundary() {
                    before = before.min(boundary);
                }
            }
            before
        });
        self.round = None;
        self.observation = None;
        // A wider observation never authorizes wider pruning. Capture the prune
        // limit when the round begins, even if advancement changes mid-round.
        self.safe = self.safe.clone().max(completed.clone().min(self.round_prune_limit.clone()));
        Some(observed.unwrap_or(completed))
    }
}

#[cfg(test)]
mod tests {
    use super::Frontier;

    #[test]
    fn admission_after_a_report_prevents_advancement_in_that_round() {
        let mut frontier = Frontier::new(2);
        frontier.request(80_u64);
        let round = frontier.begin().unwrap();
        frontier.report(round, 0, None);
        frontier.admitted(&90);

        let permission = frontier.report(round, 1, None);

        assert_eq!(permission, Some(0));
        assert_eq!(*frontier.safe(), 0);
        assert!(!frontier.measuring());
    }

    #[test]
    fn a_missing_worker_report_never_authorizes_pruning() {
        let mut frontier = Frontier::new(2);
        frontier.request(100_u64);
        let round = frontier.begin().unwrap();
        assert_eq!(frontier.report(round, 0, None), None);
        assert_eq!(*frontier.safe(), 0);
        assert_eq!(frontier.begin(), None);
        assert_eq!(frontier.report(round, 1, None), Some(100));
    }

    #[test]
    fn submissions_during_the_round_cover_work_after_a_worker_report() {
        let mut frontier = Frontier::new(2);
        frontier.request(100_u64);
        let round = frontier.begin().unwrap();
        frontier.report(round, 0, None);
        frontier.admitted(&80);
        assert_eq!(frontier.report(round, 1, Some(110)), Some(0));
        assert_eq!(*frontier.safe(), 0);
        let round = frontier.begin().unwrap();
        frontier.report(round, 0, None);
        assert_eq!(frontier.report(round, 1, None), Some(100));
    }

    #[test]
    fn duplicate_and_old_reports_cannot_complete_a_round() {
        let mut frontier = Frontier::new(2);
        frontier.request(100_u64);
        let round = frontier.begin().unwrap();
        frontier.report(round, 0, Some(60));
        frontier.report(round, 0, None);
        assert_eq!(*frontier.safe(), 0);
        assert_eq!(frontier.report(round, 1, None), Some(60));
        let next = frontier.begin().unwrap();
        assert_eq!(frontier.report(round, 1, None), None);
        assert_eq!(frontier.report(next, 0, None), None);
        assert_eq!(frontier.report(next, 1, None), Some(100));
    }

    #[test]
    fn each_round_is_capped_at_its_requested_cutoff() {
        let mut frontier = Frontier::new(1);
        frontier.request(100_u64);
        let round = frontier.begin().unwrap();
        frontier.request(200);
        assert_eq!(frontier.report(round, 0, None), Some(100));
        let round = frontier.begin().unwrap();
        frontier.request(50);
        assert_eq!(frontier.report(round, 0, Some(300)), Some(200));
        assert_eq!(frontier.begin(), None);
    }

    #[test]
    fn observation_and_pruning_use_the_same_reports_with_separate_limits() {
        let mut frontier = Frontier::new(2);
        frontier.request(100_u64);
        let round = frontier.begin_observing(Some(300)).unwrap();
        assert_eq!(frontier.begin_observing(Some(400)), None);
        assert_eq!(frontier.report(round, 0, Some(250)), None);
        assert_eq!(frontier.report(round, 1, None), Some(250));
        assert_eq!(*frontier.safe(), 100);
        assert_eq!(*frontier.requested(), 100);

        let next = frontier.begin_observing(Some(300)).unwrap();
        frontier.report(next, 0, None);
        frontier.admitted(&150);
        assert_eq!(frontier.report(next, 1, None), Some(100));
        assert_eq!(*frontier.safe(), 100);
    }

    #[test]
    fn widening_the_requested_prune_during_observation_does_not_expand_that_round() {
        let mut frontier = Frontier::new(1);
        frontier.request(100_u64);
        let round = frontier.begin_observing(Some(400)).unwrap();
        frontier.request(300);
        assert_eq!(frontier.report(round, 0, None), Some(400));
        assert_eq!(*frontier.safe(), 100);
        let next = frontier.begin().unwrap();
        assert_eq!(frontier.report(next, 0, None), Some(300));
        assert_eq!(*frontier.safe(), 300);
    }
}
