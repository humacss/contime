//! Local request adapters for caller-driven processing.
//!
//! Callbacks run synchronously during worker processing, may own thread-local
//! state, and must not block waiting for that worker. Queued requests dropped
//! before processing do not invoke callbacks. This module owns no storage or loop.
use contime_worker::{
    AdvanceInput, ApplyInput, Completion, EventQueryInput, EventQueryResponse, RoutedInput, SnapshotListenInput, SnapshotListener,
    SnapshotQueryInput, SnapshotQueryResponse, WorkInput, WorkInputKind,
};
use std::{convert::Infallible, marker::PhantomData};

/// A local callback armed only after the worker accepts its operation.
/// Empty processed results still invoke the callback once. Queued cancellation
/// and unwinding do not report an empty result as successful completion.
pub struct Reply<T: Default> {
    callback: Option<Box<dyn FnOnce(T)>>,
    armed: bool,
}
impl<T: Default> Reply<T> {
    fn new(callback: impl FnOnce(T) + 'static) -> Self {
        Self { callback: Some(Box::new(callback)), armed: false }
    }
    fn arm(mut self) -> Self {
        self.armed = true;
        self
    }
    fn deliver(mut self, value: T) {
        if let Some(callback) = self.callback.take() {
            callback(value);
        }
    }
}
impl<T: Default> Drop for Reply<T> {
    fn drop(&mut self) {
        if self.armed && !std::thread::panicking() {
            if let Some(callback) = self.callback.take() {
                callback(T::default());
            }
        }
    }
}
impl<R> Completion<R> for Reply<Vec<R>> {
    fn reject(self, rejections: Vec<R>) {
        self.deliver(rejections);
    }
}
impl<S> SnapshotQueryResponse<S> for Reply<Vec<Box<S>>> {
    fn send(self, snapshots: Vec<Box<S>>) {
        self.deliver(snapshots);
    }
}
impl<I> EventQueryResponse<I> for Reply<Vec<I>> {
    fn send(self, events: Vec<I>) {
        self.deliver(events);
    }
}

/// One local insertion request with a request-scoped rejection callback.
pub struct Apply<I, R> {
    inputs: Vec<RoutedInput<I>>,
    response: Reply<Vec<R>>,
}
impl<I, R> ApplyInput for Apply<I, R> {
    type Route = RoutedInput<I>;
    type Completion = Reply<Vec<R>>;
    fn into_parts(self) -> (Vec<Self::Route>, Self::Completion) {
        (self.inputs, self.response.arm())
    }
}
/// One local snapshot query.
pub struct Query<T, S> {
    time: T,
    ids: Vec<u128>,
    response: Reply<Vec<Box<S>>>,
}
impl<T, S> SnapshotQueryInput for Query<T, S> {
    type Time = T;
    type Response = Reply<Vec<Box<S>>>;
    fn into_parts(self) -> (T, Vec<u128>, Self::Response) {
        (self.time, self.ids, self.response.arm())
    }
}
/// One local retained-event query.
pub struct Events<T, I> {
    id: u128,
    from: T,
    to: T,
    response: Reply<Vec<I>>,
}
impl<T, I> EventQueryInput for Events<T, I> {
    type Time = T;
    type Response = Reply<Vec<I>>;
    fn into_parts(self) -> (u128, T, T, Self::Response) {
        (self.id, self.from, self.to, self.response.arm())
    }
}
/// Processing target or explicit pruning horizon; acceptance is not replay completion.
#[derive(Clone)]
pub struct Advance<T>(T);
impl<T: contime_worker::AdvanceTime> AdvanceInput for Advance<T> {
    type Time = T;
    type Completion = ();
    fn into_parts(self) -> (T, ()) {
        (self.0, ())
    }
}
/// Uninhabited listener type: this local request interface has no subscriptions.
#[derive(Clone)]
pub enum NoListener {}
impl<T> SnapshotListener<T> for NoListener {
    fn registered(&self, _: T, _: Vec<u128>) -> bool {
        match *self {}
    }
    fn replayed(&self, _: T, _: Vec<u128>) -> bool {
        match *self {}
    }
}
/// Unconstructible registration required by the worker's generic protocol.
pub struct NoRegistration<T>(Infallible, PhantomData<T>);
impl<T: Clone + Ord + Default> SnapshotListenInput for NoRegistration<T> {
    type Time = T;
    type Listener = NoListener;
    fn into_parts(self) -> (T, Vec<u128>, NoListener) {
        match self.0 {}
    }
}

enum Kind<I, S, T, R> {
    Apply(Apply<I, R>),
    Query(Query<T, S>),
    Events(Events<T, I>),
    Advance(Advance<T>),
    Prune(Advance<T>),
}
/// Opaque requests consumed by one manually driven worker.
/// These adapters require no `Send`/`Sync` bounds, channel, router or background task.
pub struct Message<I, S, T, R>(Kind<I, S, T, R>);
impl<I, S, T, R> Message<I, S, T, R> {
    /// Construct an insertion request. The callback receives all per-route
    /// rejections, or an empty vector once insertion succeeds, before live replay.
    pub fn apply(inputs: Vec<RoutedInput<I>>, callback: impl FnOnce(Vec<R>) + 'static) -> Self {
        Self(Kind::Apply(Apply { inputs, response: Reply::new(callback) }))
    }
    /// Construct a snapshot query. Missing histories are omitted; an entirely
    /// empty processed query invokes the callback with an empty vector.
    pub fn query_at(time: T, ids: Vec<u128>, callback: impl FnOnce(Vec<Box<S>>) + 'static) -> Self {
        Self(Kind::Query(Query { time, ids, response: Reply::new(callback) }))
    }
    /// Construct a retained-event query over `[from, to)`.
    pub fn query_events(id: u128, from: T, to: T, callback: impl FnOnce(Vec<I>) + 'static) -> Self {
        Self(Kind::Events(Events { id, from, to, response: Reply::new(callback) }))
    }
    /// Raise the processing target when the worker reaches this message.
    pub fn advance(time: T) -> Self {
        Self(Kind::Advance(Advance(time)))
    }
    /// Request explicit pruning after preceding replay has finished.
    pub fn prune(time: T) -> Self {
        Self(Kind::Prune(Advance(time)))
    }
}
impl<I, S, T: contime_worker::AdvanceTime, R> WorkInput for Message<I, S, T, R> {
    type Apply = Apply<I, R>;
    type SnapshotQuery = Query<T, S>;
    type EventQuery = Events<T, I>;
    type SnapshotListen = NoRegistration<T>;
    type Advance = Advance<T>;
    fn into_kind(self) -> WorkInputKind<Self::Apply, Self::SnapshotQuery, Self::EventQuery, Self::SnapshotListen, Self::Advance> {
        match self.0 {
            Kind::Apply(x) => WorkInputKind::Apply(x),
            Kind::Query(x) => WorkInputKind::SnapshotQuery(x),
            Kind::Events(x) => WorkInputKind::EventQuery(x),
            Kind::Advance(x) => WorkInputKind::Advance(x),
            Kind::Prune(x) => WorkInputKind::Prune(x),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    #[test]
    fn unprocessed_insertions_do_not_report_success_when_dropped() {
        let results = Rc::new(RefCell::new(Vec::new()));
        let output = results.clone();
        let request = Message::<u64, (), u64, &'static str>::apply(vec![RoutedInput { snapshot_id: 7, input: 1 }], move |rejections| {
            output.borrow_mut().push(rejections)
        });
        drop(request);
        assert!(results.borrow().is_empty());
    }

    #[test]
    fn accepted_insertions_report_rejections_once_or_empty_success() {
        let results = Rc::new(RefCell::new(Vec::new()));
        for rejected in [false, true] {
            let output = results.clone();
            let request = Message::<u64, (), u64, &'static str>::apply(vec![], move |values| output.borrow_mut().push(values));
            let WorkInputKind::Apply(batch) = request.into_kind() else { unreachable!() };
            let (_, completion) = batch.into_parts();
            if rejected {
                completion.reject(vec!["expired"]);
            } else {
                drop(completion);
            }
        }
        assert_eq!(*results.borrow(), [vec![], vec!["expired"]]);
    }

    #[test]
    fn unwinding_an_accepted_query_does_not_report_an_empty_success() {
        let results = Rc::new(RefCell::new(Vec::new()));
        let output = results.clone();
        let request = Message::<u64, (), u64, ()>::query_at(1, vec![7], move |states| output.borrow_mut().push(states));
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let WorkInputKind::SnapshotQuery(query) = request.into_kind() else { unreachable!() };
            let (_, _, _response) = query.into_parts();
            panic!("application query failed");
        }));
        assert!(failure.is_err());
        assert!(results.borrow().is_empty());
    }
}
