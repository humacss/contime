use crate::Route;

pub(crate) fn create_snapshot<R, S>(route: &R) -> S
where
    R: Route<S>,
    S: Default,
{
    let mut snapshot = S::default();
    route.initialize_snapshot(&mut snapshot);
    snapshot
}

#[cfg(test)]
mod tests {
    use crate::Route;

    #[derive(Debug, Default, Eq, PartialEq)]
    struct TestSnapshot {
        id: u128,
        time: i64,
    }

    struct TestEvent {
        snapshot_id: u128,
    }

    impl Route<TestSnapshot> for TestEvent {
        fn snapshot_id(&self) -> u128 {
            self.snapshot_id
        }

        fn initialize_snapshot(&self, snapshot: &mut TestSnapshot) {
            snapshot.id = self.snapshot_id;
        }
    }

    #[test]
    fn routed_creation_initializes_identity_without_advancing_default_time() {
        let event = TestEvent { snapshot_id: 41 };

        let snapshot = event.create_snapshot();

        assert_eq!(snapshot, TestSnapshot { id: 41, time: 0 });
    }
}
