use std::collections::BTreeSet;

/// A conservative admission bound replaced only after every affected route
/// reports its actual earliest possible work. Callers own delivery and locking.
pub struct CoverageConstraint<T, K> {
    fallback: T,
    pending: BTreeSet<K>,
    minimum: Option<T>,
}

impl<T: Clone + Ord, K: Ord> CoverageConstraint<T, K> {
    /// Starts with the distinct route keys whose actual work is still unknown.
    ///
    /// Until all keys are covered, `fallback` participates in the minimum. An
    /// empty route set is already covered and has no work bound.
    pub fn new(fallback: T, routes: impl IntoIterator<Item = K>) -> Self {
        Self { fallback, pending: routes.into_iter().collect(), minimum: None }
    }

    /// Unknown and duplicate routes cannot acknowledge other pending routes.
    /// `None` means this route added no work, not that other routes are covered.
    pub fn cover(&mut self, route: K, minimum: Option<T>) {
        if self.pending.remove(&route) {
            if let Some(minimum) = minimum {
                self.minimum = Some(match self.minimum.take() {
                    Some(old) => old.min(minimum),
                    None => minimum,
                });
            }
        }
    }

    /// Returns the earliest possible work across reports and uncovered routes.
    ///
    /// While any route is pending, includes `fallback` conservatively. Once every
    /// route is covered, returns the minimum reported timestamp, or `None` if no
    /// route reported work. Reading the boundary does not consume this reducer.
    pub fn boundary(&self) -> Option<T> {
        if self.pending.is_empty() {
            self.minimum.clone()
        } else {
            Some(self.minimum.as_ref().map_or_else(|| self.fallback.clone(), |minimum| minimum.min(&self.fallback).clone()))
        }
    }
}
