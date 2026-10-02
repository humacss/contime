use std::collections::BTreeSet;

/// A conservative admission bound replaced only after every affected route
/// reports its actual earliest possible work. Callers own delivery and locking.
pub struct CoverageConstraint<T, K> {
    fallback: T,
    pending: BTreeSet<K>,
    minimum: Option<T>,
}

impl<T: Clone + Ord, K: Ord> CoverageConstraint<T, K> {
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

    pub fn boundary(&self) -> Option<T> {
        if self.pending.is_empty() {
            self.minimum.clone()
        } else {
            Some(self.minimum.as_ref().map_or_else(|| self.fallback.clone(), |minimum| minimum.min(&self.fallback).clone()))
        }
    }
}
