use contime_monitor::{Monitor, Totals};
use contime_simulator::{Maximum, MaximumConfig};
use std::time::Duration;
fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
fn setup() -> (Maximum, Monitor) {
    let mut monitor = Monitor::default();
    monitor.sample(Duration::ZERO).unwrap();
    (Maximum::new(MaximumConfig { observation_interval: Duration::from_secs(1), ..MaximumConfig::default() }).unwrap(), monitor)
}
#[test]
fn slow_one_hz_query_pauses_before_completion_and_never_grows_the_rate() {
    for delay in [1151, 6600, 11900] {
        let (mut control, mut monitor) = setup();
        assert!(control.allowance(Duration::ZERO, &monitor).unwrap() > 0);
        monitor.record_submission(Totals { events: 100, applications: 100 }).unwrap();
        let query = monitor.start_query(ms(1000)).unwrap();
        let initial = control.rate();
        control.constrained();
        assert_eq!(control.allowance(ms(1051), &monitor).unwrap(), 0);
        assert!(control.rate() < initial);
        let reduced = control.rate();
        monitor.complete_query(query, ms(1000 + delay)).unwrap();
        control.allowance(ms(1000 + delay), &monitor).unwrap();
        assert_eq!(control.rate(), reduced, "late completion must not back off twice or grow");
    }
}
#[test]
fn one_prompt_observation_only_grows_once_and_no_feedback_stops_production() {
    let (mut control, mut monitor) = setup();
    monitor.record_submission(Totals { events: 100, applications: 100 }).unwrap();
    let query = monitor.start_query(ms(1000)).unwrap();
    monitor.complete_query(query, ms(1001)).unwrap();
    control.constrained();
    let initial = control.rate();
    control.allowance(ms(1001), &monitor).unwrap();
    assert_eq!(control.rate(), initial + initial / 10);
    control.constrained();
    control.allowance(ms(1002), &monitor).unwrap();
    assert_eq!(control.rate(), initial + initial / 10);
    monitor.record_submission(Totals { events: 1, applications: 1 }).unwrap();
    assert_eq!(control.allowance(ms(2100), &monitor).unwrap(), 0);
}
#[test]
fn pacing_does_not_release_an_entire_seconds_backlog_at_once() {
    let (mut control, monitor) = setup();
    let first = control.allowance(Duration::ZERO, &monitor).unwrap();
    assert_eq!(first, 200);
    control.consume(first).unwrap();
    assert_eq!(control.allowance(Duration::ZERO, &monitor).unwrap(), 0);
    assert_eq!(control.allowance(ms(1), &monitor).unwrap(), 200);
}
#[test]
fn whole_multi_target_events_fit_and_clock_reversal_is_rejected() {
    let config = MaximumConfig { minimum_work: 1000, ..MaximumConfig::default() };
    let mut control = Maximum::new(config).unwrap();
    let monitor = Monitor::default();
    assert_eq!(control.allowance(ms(10), &monitor).unwrap(), 1000);
    control.consume(1000).unwrap();
    assert!(control.allowance(ms(9), &monitor).is_err());
    assert!(control.consume(1).is_err());
}

#[test]
fn integer_credit_preserves_single_unit_debits_at_extreme_rates() {
    let mut control = Maximum::new(MaximumConfig {
        initial_rate: u64::MAX,
        maximum_rate: u64::MAX,
        maximum_outstanding: u64::MAX,
        ..MaximumConfig::default()
    })
    .unwrap();
    let monitor = Monitor::default();
    let before = control.allowance(Duration::ZERO, &monitor).unwrap();
    control.consume(1).unwrap();
    assert_eq!(control.allowance(Duration::ZERO, &monitor).unwrap(), before - 1);
}
#[test]
fn overdue_completion_with_newer_work_does_not_back_off_twice() {
    let (mut control, mut monitor) = setup();
    monitor.record_submission(Totals { events: 100, applications: 100 }).unwrap();
    let query = monitor.start_query(ms(1000)).unwrap();
    monitor.record_submission(Totals { events: 100, applications: 100 }).unwrap();
    control.allowance(ms(1051), &monitor).unwrap();
    let reduced = control.rate();
    monitor.complete_query(query, ms(7600)).unwrap();
    assert_eq!(control.allowance(ms(7600), &monitor).unwrap(), 0);
    assert_eq!(control.rate(), reduced);
}
#[test]
fn prompt_partial_query_does_not_hide_previous_period_backlog() {
    let (mut control, mut monitor) = setup();
    monitor.record_submission(Totals { events: 100, applications: 200 }).unwrap();
    monitor.sample(ms(1000)).unwrap();
    let query = monitor.start_query_through(ms(1000), Totals { events: 50, applications: 100 }).unwrap();
    monitor.complete_query(query, ms(1001)).unwrap();
    control.constrained();
    let initial = control.rate();
    control.allowance(ms(1001), &monitor).unwrap();
    assert!(control.rate() < initial);
}
#[test]
fn small_rates_still_back_off_and_invalid_configuration_is_rejected() {
    let mut control = Maximum::new(MaximumConfig { initial_rate: 4, ..MaximumConfig::default() }).unwrap();
    let mut monitor = Monitor::default();
    monitor.record_submission(Totals { events: 1, applications: 1 }).unwrap();
    monitor.start_query(Duration::ZERO).unwrap();
    assert_eq!(control.allowance(ms(51), &monitor).unwrap(), 0);
    assert_eq!(control.rate(), 3);
    assert!(Maximum::new(MaximumConfig { minimum_work: 11, maximum_outstanding: 10, ..MaximumConfig::default() }).is_err());
}
