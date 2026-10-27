use contime_monitor::{ApplicationActivity, WorkerReading};
use std::sync::Arc;

/// Backend-independent navigation. Map host keyboard or mouse events to these.
#[derive(Clone, Copy, Debug)]
pub enum Scroll {
    /// Previous worker row.
    Up,
    /// Next worker row.
    Down,
    /// Previous visible page.
    PageUp,
    /// Next visible page.
    PageDown,
    /// First worker.
    Home,
    /// Last visible page.
    End,
}

/// Presentation state for one immutable worker reading.
///
/// Collection remains outside this type. Updating scans worker gauges once;
/// drawing formats only visible rows and columns. Timestamps use the host's
/// `Display` implementation. Creating or drawing a view starts no threads,
/// reads no clock, and performs no terminal I/O itself.
pub struct WorkerView<T> {
    pub(super) reading: Option<Arc<WorkerReading<T>>>,
    pub(super) totals: [u128; 3],
    pub(super) activity: Option<ApplicationActivity>,
    pub(super) baseline: Option<Arc<WorkerReading<T>>>,
    pub(super) average: Option<ApplicationActivity>,
    pub(super) counted: bool,
    pub(super) offset: usize,
    pub(super) page: usize,
    pub(super) error: Option<String>,
}
impl<T> Default for WorkerView<T> {
    fn default() -> Self {
        Self {
            reading: None,
            baseline: None,
            average: None,
            activity: None,
            counted: false,
            totals: [0; 3],
            offset: 0,
            page: 1,
            error: None,
        }
    }
}
impl<T> WorkerView<T> {
    /// Installs a complete immutable reading and caches summary totals.
    /// Previous errors remain visible until explicitly cleared by the host.
    /// Average rates cover the interval since the first reading (subtracting its
    /// counters); current rates cover the latest interval. Create a new view when
    /// replacing the runtime source. Counter resets restart the average baseline.
    pub fn set_reading(&mut self, reading: Arc<WorkerReading<T>>) {
        self.totals = reading.workers.iter().fold([0u128; 3], |mut sums, worker| {
            for (sum, value) in sums.iter_mut().zip([worker.history_stores, worker.pending_histories, worker.due_histories]) {
                *sum = sum.saturating_add(value as u128);
            }
            sums
        });
        self.activity = Some(ApplicationActivity::between(self.reading.as_deref(), &reading));
        // Reuse the same counter validity rules for interval and cumulative rates.
        // A reset or changed worker set starts a fresh observation interval.
        let restart = self.reading.is_some() && self.activity.as_ref().is_some_and(|activity| activity.inserted_per_second.is_none());
        if self.baseline.is_none() || restart {
            self.baseline = Some(Arc::clone(&reading));
        }
        self.average = Some(ApplicationActivity::between(self.baseline.as_deref(), &reading));
        self.counted = reading.workers.iter().any(|worker| worker.counters.is_some());
        self.reading = Some(reading);
        self.clamp();
    }
    /// Displays a caller-provided collection error without discarding good data.
    /// Pass `None` when the host considers collection healthy again.
    pub fn set_error(&mut self, error: Option<String>) {
        self.error = error;
    }
    /// Scrolls worker rows. Page size follows the last rendered viewport.
    pub fn scroll(&mut self, scroll: Scroll) {
        self.offset = match scroll {
            Scroll::Up => self.offset.saturating_sub(1),
            Scroll::Down => self.offset.saturating_add(1),
            Scroll::PageUp => self.offset.saturating_sub(self.page),
            Scroll::PageDown => self.offset.saturating_add(self.page),
            Scroll::Home => 0,
            Scroll::End => usize::MAX,
        };
        self.clamp();
    }
    /// Index of the first visible worker in reading order.
    pub fn offset(&self) -> usize {
        self.offset
    }
    pub(super) fn clamp(&mut self) {
        let count = self.reading.as_ref().map_or(0, |reading| reading.workers.len());
        self.offset = self.offset.min(count.saturating_sub(self.page));
    }
}
