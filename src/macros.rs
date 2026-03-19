/// Generates `SnapshotLanes` and `EventLanes` wrapper enums that implement the
/// contime lane traits, plus a `Contime` type alias.
///
/// # Single-snapshot form
///
/// ```ignore
/// contime::contime! { MySnapshot { MyEvent } }
/// // or with a named module:
/// contime::contime! { mod my_mod; MySnapshot { MyEvent } }
/// ```
///
/// # Multi-snapshot form
///
/// List snapshot types, then events with explicit variant names, types, and
/// target snapshots. Events shared across snapshots are listed once with all
/// targets.
///
/// ```ignore
/// contime::contime! {
///     mod state;
///     snapshots { AccountAt, ActorAt, CharacterAt, ConnectionAt },
///     OnAccountCreated(Event<OnAccountCreated>) => [AccountAt],
///     OnCharacterCreated(Event<OnCharacterCreated>) => [AccountAt, CharacterAt],
///     OnNewActorDeltas(Event<OnNewActorDeltas>) => [ActorAt],
///     OnConnectionServerTime(Event<OnConnectionServerTime>) => [ConnectionAt],
/// }
/// ```
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

    // ── Single-snapshot with module name ────────────────────────────────
    (mod $modname:ident; $snapshot:ident { $event:ident $(,)? }) => {
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
                            Self::$snapshot(<$snapshot as $crate::Snapshot>::from_event(event))
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

            impl $crate::ApplyEvent<SnapshotLanes> for EventLanes {
                fn snapshot_id(&self) -> u128 {
                    match self {
                        EventLanes::$event(event) => {
                            <$event as $crate::ApplyEvent<$snapshot>>::snapshot_id(event)
                        }
                    }
                }

                fn apply_to(&self, snapshot: &mut SnapshotLanes) {
                    match self {
                        Self::$event(event) => {
                            match snapshot {
                                SnapshotLanes::$snapshot(s) => {
                                    <$event as $crate::ApplyEvent<$snapshot>>::apply_to(event, s)
                                }
                            }
                        }
                    }
                }
            }

            impl $crate::EventLanes<SnapshotLanes> for EventLanes {
                fn snapshots(&self) -> Vec<SnapshotLanes> {
                    match self {
                        Self::$event(event) => {
                            vec![<$snapshot as $crate::Snapshot>::from_event(event).into()]
                        }
                    }
                }
            }

            impl From<$event> for EventLanes {
                fn from(event: $event) -> Self {
                    Self::$event(event)
                }
            }

            pub type Contime = $crate::Contime<SnapshotLanes, EventLanes>;
        }
    };

    // ── Multi-snapshot form ─────────────────────────────────────────────
    //
    // Syntax:
    //   contime! {
    //       mod my_module;
    //       snapshots { SnapA, SnapB },
    //       VariantX($TypeX) => [SnapA],
    //       VariantY($TypeY) => [SnapA, SnapB],
    //   }
    //
    // $variant is the enum variant name used in both EventLanes and the
    // per-snapshot event enums. $type is the concrete type stored in the
    // variant. These are explicit — no magic name matching.
    (
        mod $modname:ident;
        snapshots { $( $snapshot:ident ),+ $(,)? },
        $( $variant:ident ( $evtype:ty ) => [ $( $target:ident ),+ $(,)? ] ),+ $(,)?
    ) => {
        mod $modname {
            use super::*;

            // ── SnapshotLanes enum ─────────────────────────────────────
            #[derive(Clone, Debug, PartialEq, Eq)]
            pub enum SnapshotLanes {
                $( $snapshot($snapshot), )+
            }

            impl $crate::SnapshotLanes for SnapshotLanes {}

            impl $crate::Snapshot for SnapshotLanes {
                type Event = EventLanes;

                fn id(&self) -> u128 {
                    match self {
                        $( Self::$snapshot(s) => <$snapshot as $crate::Snapshot>::id(s), )+
                    }
                }
                fn time(&self) -> i64 {
                    match self {
                        $( Self::$snapshot(s) => <$snapshot as $crate::Snapshot>::time(s), )+
                    }
                }
                fn set_time(&mut self, time: i64) {
                    match self {
                        $( Self::$snapshot(s) => <$snapshot as $crate::Snapshot>::set_time(s, time), )+
                    }
                }
                fn conservative_size(&self) -> u64 {
                    match self {
                        $( Self::$snapshot(s) => <$snapshot as $crate::Snapshot>::conservative_size(s), )+
                    }
                }

                fn from_event(event: &Self::Event) -> Self {
                    match event {
                        $(
                            EventLanes::$variant(e) => {
                                // Convert individual event into the first target's
                                // event enum via From, then call from_event
                                $crate::__contime_first_from_event!(e; $variant; $( $target ),+ )
                            }
                        )+
                    }
                }
            }

            // From/TryFrom conversions for each concrete snapshot type
            $(
                impl From<$snapshot> for SnapshotLanes {
                    fn from(snapshot: $snapshot) -> Self {
                        Self::$snapshot(snapshot)
                    }
                }

                impl TryFrom<SnapshotLanes> for $snapshot {
                    type Error = SnapshotLanes;
                    fn try_from(lane: SnapshotLanes) -> Result<Self, Self::Error> {
                        match lane {
                            SnapshotLanes::$snapshot(s) => Ok(s),
                            other => Err(other),
                        }
                    }
                }
            )+

            // ── EventLanes enum ────────────────────────────────────────
            #[derive(Debug, Clone, Eq, PartialEq)]
            pub enum EventLanes {
                $( $variant($evtype), )+
            }

            // ── Event trait ────────────────────────────────────────────
            impl $crate::Event for EventLanes {
                fn id(&self) -> u128 {
                    match self {
                        $( Self::$variant(e) => <$evtype as $crate::Event>::id(e), )+
                    }
                }
                fn time(&self) -> i64 {
                    match self {
                        $( Self::$variant(e) => <$evtype as $crate::Event>::time(e), )+
                    }
                }
                fn conservative_size(&self) -> u64 {
                    match self {
                        $( Self::$variant(e) => <$evtype as $crate::Event>::conservative_size(e), )+
                    }
                }
            }

            // ── ApplyEvent<SnapshotLanes> ──────────────────────────────
            impl $crate::ApplyEvent<SnapshotLanes> for EventLanes {
                fn snapshot_id(&self) -> u128 {
                    match self {
                        $(
                            Self::$variant(e) => {
                                $crate::__contime_first_snapshot_id_typed!(e; $evtype; $( $target ),+ )
                            }
                        )+
                    }
                }

                fn apply_to(&self, snapshot: &mut SnapshotLanes) {
                    match (self, snapshot) {
                        $($(
                            (Self::$variant(e), SnapshotLanes::$target(s)) => {
                                <$evtype as $crate::ApplyEvent<$target>>::apply_to(e, s)
                            }
                        )+)+
                        #[allow(unreachable_patterns)]
                        _ => {} // Event-snapshot combination not mapped
                    }
                }
            }

            // ── EventLanes<SnapshotLanes> ──────────────────────────────
            impl $crate::EventLanes<SnapshotLanes> for EventLanes {
                fn snapshots(&self) -> Vec<SnapshotLanes> {
                    match self {
                        $(
                            Self::$variant(e) => {
                                vec![
                                    $(
                                        {
                                            let snapshot_event: <$target as $crate::Snapshot>::Event =
                                                <_ as From<$evtype>>::from(e.clone());
                                            SnapshotLanes::$target(
                                                <$target as $crate::Snapshot>::from_event(&snapshot_event)
                                            )
                                        },
                                    )+
                                ]
                            }
                        )+
                    }
                }
            }

            // ── From<$evtype> for EventLanes ───────────────────────────
            $(
                impl From<$evtype> for EventLanes {
                    fn from(event: $evtype) -> Self {
                        Self::$variant(event)
                    }
                }
            )+

            // ── From<$evtype> for each target's Snapshot::Event ────────
            // Uses the $variant ident as the enum variant name in the
            // target's event enum.
            $($(
                impl From<$evtype> for <$target as $crate::Snapshot>::Event {
                    fn from(e: $evtype) -> Self {
                        Self::$variant(e)
                    }
                }
            )+)+

            pub type Contime = $crate::Contime<SnapshotLanes, EventLanes>;
        }
    };
}

/// Internal helper: calls from_event on the first target snapshot type.
#[macro_export]
#[doc(hidden)]
macro_rules! __contime_first_from_event {
    ($event:expr; $variant:ident; $first:ident $(, $rest:ident )* ) => {
        {
            let snapshot_event: <$first as $crate::Snapshot>::Event =
                <_ as From<_>>::from(($event).clone());
            SnapshotLanes::$first(<$first as $crate::Snapshot>::from_event(&snapshot_event))
        }
    };
}

/// Internal helper: returns snapshot_id from the first target.
#[macro_export]
#[doc(hidden)]
macro_rules! __contime_first_snapshot_id_typed {
    ($event:expr; $evtype:ty; $first:ident $(, $rest:ident )* ) => {
        <$evtype as $crate::ApplyEvent<$first>>::snapshot_id($event)
    };
}
