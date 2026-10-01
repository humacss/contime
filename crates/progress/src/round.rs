use std::num::NonZeroUsize;

/// Collects one fenced measurement from a fixed set of participants.
///
/// A result `T` bounds work strictly before `T`, not the bucket at `T`.
/// This type does not establish fences or inspect queues. The caller must
/// provide ordered reports, account for work admitted during measurement,
/// and resolve participants after completion. It never prunes or rejects input.
pub struct MeasurementRound<T> {
    id: u64,
    minimum: T,
    reported: Vec<bool>,
    remaining: usize,
}

impl<T: Clone + Ord> MeasurementRound<T> {
    /// Participants are numbered `0..participants`; `id` distinguishes rounds.
    pub fn new(id: u64, cutoff: T, participants: NonZeroUsize) -> Self {
        Self { id, minimum: cutoff, reported: vec![false; participants.get()], remaining: participants.get() }
    }

    pub fn is_complete(&self) -> bool {
        self.remaining == 0
    }

    /// Caps this observation, for example at the previous committed boundary
    /// when an admission can restart earlier replay. Never raises the result.
    pub fn constrain(&mut self, boundary: T) {
        if !self.is_complete() && boundary < self.minimum {
            self.minimum = boundary;
        }
    }

    /// `None` means the participant has no remaining work. Missing, duplicate,
    /// unknown-participant and wrong-round reports never imply completion.
    /// Returns the completed observation exactly once, on the final report.
    pub fn report(&mut self, round: u64, participant: usize, minimum: Option<T>) -> Option<T> {
        if round != self.id || self.reported.get(participant).copied().unwrap_or(true) {
            return None;
        }
        if let Some(time) = minimum {
            self.constrain(time);
        }
        self.reported[participant] = true;
        self.remaining -= 1;
        self.is_complete().then(|| self.minimum.clone())
    }
}
