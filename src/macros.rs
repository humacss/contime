/// Generates `SnapshotLanes` and `EventLanes` wrapper enums that implement the
/// contime lane traits, plus a `Contime` type alias.
///
/// # Single-snapshot form
///
/// ```ignore
/// contime::contime! { MySnapshot { MyEvent } }
/// // or with a named module:
/// contime::contime! { mod my_mod; MySnapshot { MyEvent } }
/// // with explicit context:
/// contime::contime! { mod my_mod; context MyStreamContext; MySnapshot { MyEvent } }
/// ```
///
/// # Multi-snapshot form
///
/// (unchanged)
#[macro_export]
macro_rules! contime {
    // ── Single-snapshot shorthand (no module name) ──────────────────────
    ($snapshot:ident { $event:ident $(,)? }) => {
        $crate::contime! {
            mod __contime;
            $snapshot {
                $event,
            }
        }
    };

    // ── Single-snapshot with module name + OPTIONAL context ─────────────
    (mod $modname:ident; context $context:ty; $snapshot:ident { $event:ident $(,)? }) => {
        mod $modname {
            use super::*;
            #[derive(Clone, Debug, PartialEq, Eq)]
            pub enum SnapshotLanes {
                $snapshot($snapshot),
            }
            impl $crate::SnapshotLanes for SnapshotLanes {}

            impl $crate::Snapshot for SnapshotLanes {
                type Event = EventLanes;
                fn id(&self) -> u128 {
                    match self {
                        Self::$snapshot(s) => <$snapshot as $crate::Snapshot>::id(s),
                    }
                }
                fn time(&self) -> i64 {
                    match self {
                        Self::$snapshot(s) => <$snapshot as $crate::Snapshot>::time(s),
                    }
                }
                fn set_time(&mut self, time: i64) {
                    match self {
                        Self::$snapshot(s) => <$snapshot as $crate::Snapshot>::set_time(s, time),
                    }
                }
                fn conservative_size(&self) -> u64 {
                    match self {
                        Self::$snapshot(s) => <$snapshot as $crate::Snapshot>::conservative_size(s),
                    }
                }
                fn from_event(event: &Self::Event) -> Self {
                    match event {
                        EventLanes::$event(event) => {
                            Self::$snapshot(<$snapshot as $crate::SeedSnapshot<$event>>::seed_from_event(event))
                        }
                    }
                }
            }

            impl From<$snapshot> for SnapshotLanes {
                fn from(snapshot: $snapshot) -> Self {
                    Self::$snapshot(snapshot)
                }
            }

            impl From<SnapshotLanes> for $snapshot {
                fn from(snapshot_lane: SnapshotLanes) -> Self {
                    match snapshot_lane {
                        SnapshotLanes::$snapshot(snapshot) => snapshot,
                    }
                }
            }

            #[derive(Debug, Clone, Eq, PartialEq)]
            pub enum EventLanes {
                $event($event),
            }

            impl $crate::Event for EventLanes {
                fn id(&self) -> u128 {
                    match self {
                        Self::$event(event) => <$event as $crate::Event>::id(event),
                    }
                }
                fn time(&self) -> i64 {
                    match self {
                        Self::$event(event) => <$event as $crate::Event>::time(event),
                    }
                }
                fn conservative_size(&self) -> u64 {
                    match self {
                        Self::$event(event) => <$event as $crate::Event>::conservative_size(event),
                    }
                }
            }

            impl $crate::SnapshotEvent<SnapshotLanes> for EventLanes
            where
                $event: $crate::SnapshotEvent<$snapshot>,
            {
                fn snapshot_id(&self) -> u128 {
                    match self {
                        EventLanes::$event(event) => {
                            <$event as $crate::SnapshotEvent<$snapshot>>::snapshot_id(event)
                        }
                    }
                }
            }

            impl $crate::ApplyEvents for SnapshotLanes
            where
                $snapshot: $crate::ApplyEvents,
                <$snapshot as $crate::Snapshot>::Event: From<$event>,
            {
                fn apply_events(&mut self, batch: $crate::ApplyBatch<'_, Self::Event>) {
                    match self {
                        SnapshotLanes::$snapshot(snapshot) => {
                            let mut bucket = Vec::new();
                            for event in batch.events {
                                match event {
                                    EventLanes::$event(event) => bucket.push(event.clone().into()),
                                }
                            }
                            <$snapshot as $crate::ApplyEvents>::apply_events(
                                snapshot,
                                $crate::ApplyBatch {
                                    snapshot_id: batch.snapshot_id,
                                    time: batch.time,
                                    events: &bucket,
                                    bucket_revision: batch.bucket_revision,
                                },
                            );
                        }
                    }
                }
            }

            // Generic over C so it works with any context (including StreamContext)
            impl<C> $crate::AfterApplyEvents<C> for SnapshotLanes
            where
                $snapshot: $crate::AfterApplyEvents<C>,
                <$snapshot as $crate::Snapshot>::Event: From<$event>,
            {
                fn after_apply_events(&self, batch: $crate::ApplyBatch<'_, Self::Event>, context: &mut C) {
                    match self {
                        SnapshotLanes::$snapshot(snapshot) => {
                            let mut bucket = Vec::new();
                            for event in batch.events {
                                match event {
                                    EventLanes::$event(event) => bucket.push(event.clone().into()),
                                }
                            }
                            <$snapshot as $crate::AfterApplyEvents<C>>::after_apply_events(
                                snapshot,
                                $crate::ApplyBatch {
                                    snapshot_id: batch.snapshot_id,
                                    time: batch.time,
                                    events: &bucket,
                                    bucket_revision: batch.bucket_revision,
                                },
                                context,
                            );
                        }
                    }
                }
            }

            impl<C> $crate::EventLanes<SnapshotLanes, C> for EventLanes
            where
                EventLanes: $crate::SnapshotEvent<SnapshotLanes>,
                SnapshotLanes: $crate::ApplyEvents,
            {
                fn snapshots(&self) -> Vec<SnapshotLanes> {
                    match self {
                        Self::$event(event) => {
                            vec![<$snapshot as $crate::SeedSnapshot<$event>>::seed_from_event(event).into()]
                        }
                    }
                }
                fn routed_snapshots(&self) -> Vec<$crate::RoutedSnapshot<SnapshotLanes>> {
                    match self {
                        Self::$event(event) => {
                            vec![$crate::RoutedSnapshot {
                                snapshot_id: <$event as $crate::SnapshotEvent<$snapshot>>::snapshot_id(event),
                                initial_snapshot: <$snapshot as $crate::SeedSnapshot<$event>>::seed_from_event(event).into(),
                            }]
                        }
                    }
                }
            }

            impl From<$event> for EventLanes {
                fn from(event: $event) -> Self {
                    Self::$event(event)
                }
            }

            // === KEY CHANGE: Contime now includes the context type ===
            pub type Contime = $crate::Contime<SnapshotLanes, EventLanes, $context>;
        }
    };

    // ── Single-snapshot with module name (NO context → default to ()) ───
    (mod $modname:ident; $snapshot:ident { $event:ident $(,)? }) => {
        $crate::contime! {
            mod $modname;
            context ();
            $snapshot { $event }
        }
    };

    // ── Multi-snapshot form (unchanged for now) ─────────────────────────
    (
        mod $modname:ident;
        snapshots { $( $snapshot:ident ),+ $(,)? },
        $( $variant:ident ( $evtype:ty ) => [ $( $target:ident ),+ $(,)? ] ),+ $(,)?
    ) => {
        $crate::__lanes_merge! {
            mod $modname;
            snapshots { $( $snapshot ),+ }
            routes {
                $(
                    $variant($evtype) => [ $( $target ),+ ],
                )+
            }
        }
    };
}

/// Internal helper: seeds the first target snapshot type.
#[macro_export]
#[doc(hidden)]
macro_rules! __contime_first_seed_from_event {
    ($event:expr; $event_ty:ty; $first:ident $(, $rest:ident )* ) => {{
        SnapshotLanes::$first(<$first as $crate::SeedSnapshot<$event_ty>>::seed_from_event($event))
    }};
}

/// Internal helper: returns snapshot_id from the first target.
#[macro_export]
#[doc(hidden)]
macro_rules! __contime_first_snapshot_id_typed {
    ($event:expr; $evtype:ty; $first:ident $(, $rest:ident )* ) => {
        <$evtype as $crate::SnapshotEvent<$first>>::snapshot_id($event)
    };
}

/// Merges multiple compile-time fragments into one final lane universe.
///
/// `lanes!` expands the listed fragment macros, merges the collected snapshot
/// and route manifests, and generates one module-scoped `SnapshotLanes`,
/// `EventLanes`, and `Contime` alias.
///
/// ```ignore
/// use crate::snapshots::consumer_source_fragment;
/// use crate::triggers::consumer_trigger_fragment;
///
/// contime::lanes! {
///     mod example_contime;
///     fragments [
///         consumer_source_fragment,
///         consumer_trigger_fragment,
///     ];
/// }
/// ```
#[macro_export]
macro_rules! lanes {
    (
        mod $modname:ident;
        fragments [ $( $fragment:ident ),+ $(,)? ];
    ) => {
        ::contime::__contime_collect_fragments! {
            @collect
            mod $modname;
            snapshots {}
            routes {}
            fragments [ $( $fragment ),+ ]
        }
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! __contime_collect_fragments {
    (
        @collect
        mod $modname:ident;
        snapshots { $( $snapshot:tt )* }
        routes { $( $route:tt )* }
        fragments [ ]
    ) => {
        ::contime::__lanes_merge! {
            mod $modname;
            snapshots { $( $snapshot )* }
            routes { $( $route )* }
        }
    };

    (
        @collect
        mod $modname:ident;
        snapshots { $( $snapshot:tt )* }
        routes { $( $route:tt )* }
        fragments [ $next:ident $(, $rest:ident )* $(,)? ]
    ) => {
        $next! {
            @append
            mod $modname;
            snapshots { $( $snapshot )* }
            routes { $( $route )* }
            fragments [ $( $rest ),* ]
        }
    };
}
