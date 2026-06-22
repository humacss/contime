#[test]
fn public_contime_api_excludes_apply_wrapper_private_symbols() {
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs")).unwrap();

    for forbidden in [
        "pub type ScheduleKey",
        "pub use replay",
        "ReplayApplyContext",
        "ReplayEventBatch",
        "ReplayEventEmitter",
        "RoutedReplayEventEmitter",
        "AfterReplayEvents",
        "CancelScheduleHandle",
        "ScheduleHandle",
    ] {
        assert!(!source.contains(forbidden), "public contime lib.rs should not expose private apply-wrapper symbol `{forbidden}`");
    }
}
