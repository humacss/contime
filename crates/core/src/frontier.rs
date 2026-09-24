/// Coordinator-owned conservative boundary. A round accounts for work admitted
/// during measurement as well as each worker's earliest possible publication.
pub(crate) struct Frontier<T> {
    safe: T,
    requested: T,
    workers: usize,
    sequence: u64,
    round: Option<Round<T>>,
}

struct Round<T> {
    id: u64,
    minimum: T,
    reports: Vec<Option<Option<T>>>,
}

impl<T: Clone + Default + Ord> Frontier<T> {
    pub(crate) fn new(workers: usize) -> Self {
        assert!(workers > 0);
        Self { safe: T::default(), requested: T::default(), workers, sequence: 0, round: None }
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
        if self.round.is_some() || self.requested <= self.safe {
            return None;
        }
        self.sequence = self.sequence.checked_add(1).expect("measurement round overflow");
        self.round = Some(Round { id: self.sequence, minimum: self.requested.clone(), reports: vec![None; self.workers] });
        Some(self.sequence)
    }

    pub(crate) fn admitted(&mut self, _time: &T) {
        if let Some(round) = &mut self.round {
            // An insertion can reconstruct a prefix earlier than its own time.
            // Only a subsequent worker measurement can bound that replay.
            round.minimum = self.safe.clone();
        }
    }

    pub(crate) fn report(&mut self, id: u64, worker: usize, minimum: Option<T>) -> Option<T> {
        let round = self.round.as_mut()?;
        if id != round.id || worker >= self.workers || round.reports[worker].is_some() {
            return None;
        }
        if let Some(time) = &minimum {
            if *time < round.minimum {
                round.minimum = time.clone();
            }
        }
        round.reports[worker] = Some(minimum);
        if round.reports.iter().any(Option::is_none) {
            return None;
        }
        let completed = self.round.take().expect("completed round exists");
        if completed.minimum <= self.safe {
            return None;
        }
        self.safe = completed.minimum;
        Some(self.safe.clone())
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

        assert_eq!(permission, None);
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
        assert_eq!(frontier.report(round, 1, Some(110)), None);
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
}
