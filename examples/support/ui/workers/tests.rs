use super::{Scroll, WorkerView};
use contime_monitor::{WorkerReading, WorkerStatus};
use ratatui::{backend::TestBackend, Terminal};
use std::{
    fmt,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
fn reading(count: usize) -> Arc<WorkerReading<u64>> {
    Arc::new(WorkerReading {
        started_at: Duration::from_millis(2),
        completed_at: Duration::from_millis(5),
        workers: (0..count)
            .map(|worker_id| WorkerStatus {
                worker_id,
                history_stores: 1234,
                metadata_only: 2,
                pending_histories: 20,
                due_histories: 3,
                processing_target: 42,
                admission_horizon: 12,
                pending_prune_passes: 1,
                awaiting_resolution: Some(7),
                counters: None,
            })
            .collect(),
    })
}
fn compact(text: String) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn draw<T: fmt::Display>(view: &mut WorkerView<T>, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| view.render(frame, frame.area(), Duration::from_millis(15))).unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(width as usize)
        .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
#[test]
fn waiting_error_and_last_good_reading_are_distinct() {
    let mut view = WorkerView::<u64>::default();
    assert!(draw(&mut view, 160, 24).contains("Waiting for a complete worker reading"));
    view.set_reading(reading(4));
    let text = compact(draw(&mut view, 160, 24));
    for expected in [
        "4 workers",
        "4,936",
        "80 pending",
        "12 due",
        "1,234",
        "Metadata",
        "Target",
        "Horizon",
        "Prune",
        "Round",
        "42",
        "12",
        "Age: 10.000 ms",
        "Collection: 3.000 ms",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    view.set_error(Some("Worker collection failed".into()));
    let error = draw(&mut view, 160, 24);
    assert!(error.contains("Worker collection failed"));
    assert!(error.contains("1,234"));
    view.set_error(None);
    assert!(!draw(&mut view, 160, 24).contains("Worker collection failed"));
}
#[test]
fn scrolling_resize_and_shrinking_readings_keep_valid_visible_rows() {
    let mut view = WorkerView::default();
    view.set_reading(reading(100));
    draw(&mut view, 160, 20);
    view.scroll(Scroll::End);
    let end = draw(&mut view, 160, 20);
    assert!(end.contains("100 of 100"));
    assert!(view.offset() > 0);
    view.scroll(Scroll::PageUp);
    assert!(!draw(&mut view, 160, 20).contains("100 of 100"));
    view.scroll(Scroll::Home);
    assert_eq!(view.offset(), 0);
    view.scroll(Scroll::Down);
    assert_eq!(view.offset(), 1);
    view.scroll(Scroll::Up);
    assert_eq!(view.offset(), 0);
    view.scroll(Scroll::PageDown);
    assert!(view.offset() > 0);
    view.set_reading(reading(1));
    draw(&mut view, 80, 40);
    assert_eq!(view.offset(), 0);
    view.set_reading(reading(0));
    assert!(draw(&mut view, 160, 24).contains("No workers in this reading"));
}
#[test]
fn small_and_narrow_terminals_are_supported() {
    let mut view = WorkerView::default();
    view.set_reading(reading(1));
    for (w, h) in [(1, 1), (20, 5), (48, 12), (80, 24), (120, 24), (200, 40)] {
        let text = draw(&mut view, w, h);
        if w < 48 {
            assert!(text.contains("Enlarge") || w == 1);
        } else {
            assert!(text.contains("Pending"));
        }
    }
}
struct Clock(Arc<AtomicUsize>);
impl fmt::Display for Clock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fetch_add(1, Ordering::Relaxed);
        write!(f, "42")
    }
}
#[test]
fn formatting_is_limited_to_visible_rows_and_visible_columns() {
    let calls = Arc::new(AtomicUsize::new(0));
    let data = Arc::new(WorkerReading {
        started_at: Duration::ZERO,
        completed_at: Duration::ZERO,
        workers: (0..10000)
            .map(|worker_id| WorkerStatus {
                worker_id,
                history_stores: 1,
                metadata_only: 0,
                pending_histories: 1,
                due_histories: 0,
                processing_target: Clock(calls.clone()),
                admission_horizon: Clock(calls.clone()),
                pending_prune_passes: 0,
                awaiting_resolution: None,
                counters: None,
            })
            .collect(),
    });
    let mut view = WorkerView::default();
    view.set_reading(data);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    draw(&mut view, 80, 24);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    draw(&mut view, 160, 24);
    assert!(calls.load(Ordering::Relaxed) > 0);
    assert!(calls.load(Ordering::Relaxed) <= 48);
}
#[test]
fn totals_do_not_overflow_on_large_reports() {
    let mut r = reading(2);
    for worker in &mut Arc::get_mut(&mut r).unwrap().workers {
        worker.history_stores = usize::MAX;
        worker.pending_histories = usize::MAX;
    }
    let mut view = WorkerView::default();
    view.set_reading(r);
    let text = draw(&mut view, 200, 24);
    #[cfg(target_pointer_width = "64")]
    assert!(text.contains("36,893,488,147,419,103,230 snapshots"));
    #[cfg(target_pointer_width = "32")]
    assert!(text.contains("8,589,934,590 snapshots"));
}

#[test]
fn collection_and_presentation_communicate_only_through_a_complete_reading() {
    use contime_monitor::{WorkerMonitor, WorkerMonitorError, WorkerStatusSource};
    struct Ready;
    impl WorkerStatusSource<u64> for Ready {
        fn worker_count(&self) -> usize {
            4
        }
        fn request_worker_status(&self) -> Result<crossbeam_channel::Receiver<WorkerStatus<u64>>, WorkerMonitorError> {
            let (sender, receiver) = crossbeam_channel::unbounded();
            for status in &reading(4).workers {
                sender.send(status.clone()).unwrap();
            }
            Ok(receiver)
        }
    }
    let mut collector = WorkerMonitor::attach(&Ready);
    collector.begin(Duration::from_millis(2)).unwrap();
    let complete = collector.poll(Duration::from_millis(5)).unwrap().unwrap();
    let mut view = WorkerView::default();
    view.set_reading(complete);
    drop(collector);
    assert!(draw(&mut view, 160, 24).contains("4,936 snapshots"));
}

#[test]
fn viewport_inside_a_larger_frame_keeps_other_content_intact() {
    use ratatui::{layout::Rect, widgets::Paragraph};
    let mut view = WorkerView::default();
    view.set_reading(reading(2));
    let mut terminal = Terminal::new(TestBackend::new(200, 40)).unwrap();
    terminal
        .draw(|frame| {
            frame.render_widget(Paragraph::new("Host application"), Rect::new(0, 0, 200, 1));
            view.render(frame, Rect::new(5, 3, 160, 24), Duration::ZERO);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(0, 0)].symbol(), "H");
    assert_eq!(buffer[(5, 3)].symbol(), "C");
    assert_eq!(buffer[(0, 3)].symbol(), " ");
}

#[test]
fn wide_counts_never_silently_lose_their_magnitude() {
    let mut reading = reading(1);
    let worker = &mut Arc::get_mut(&mut reading).unwrap().workers[0];
    worker.due_histories = 1_000_000_000;
    worker.pending_histories = 1_000_000_000;
    worker.pending_prune_passes = 1_000_000_000;
    worker.awaiting_resolution = Some(1_000_000_000);
    let mut view = WorkerView::default();
    view.set_reading(reading);
    let text = draw(&mut view, 160, 24);
    assert!(text.contains("1,000,000…"), "Due must be visibly marked when clipped: {text}");
    assert!(text.contains("1,000,0…"), "Prune must be visibly marked when clipped: {text}");
}

#[cfg(feature = "runtime")]
#[path = "tests/model.rs"]
mod native;
#[cfg(feature = "runtime")]
#[test]
fn native_worker_reports_render_without_advancing_or_querying() {
    for workers in [1, 4] {
        use contime_runtime::{checkpoints::CheckpointConfig, ConTime, ConTimeConfig, Placement};
        let runtime = ConTime::<native::Increment, native::Counter, ()>::start(
            ConTimeConfig {
                worker_count: workers,
                router_count: 1,
                placement: Placement::default(),
                history_retention: native::Time(10_000),
                pruning_interval: Duration::from_secs(60),
                checkpoints: CheckpointConfig { interval: 128 },
            },
            (),
        )
        .unwrap();
        let (sender, receiver) = crossbeam_channel::unbounded();
        runtime.send((0..128).map(|id| native::Increment { id, time: native::Time(id as u64 + 1) }), sender).unwrap();
        assert_eq!(receiver.recv_timeout(Duration::from_secs(2)), Err(crossbeam_channel::RecvTimeoutError::Disconnected));
        let responses = runtime.request_worker_status().unwrap();
        let mut reports = (0..workers).map(|_| responses.recv_timeout(Duration::from_secs(2)).unwrap()).collect::<Vec<_>>();
        assert_eq!(responses.recv_timeout(Duration::from_secs(2)), Err(crossbeam_channel::RecvTimeoutError::Disconnected));
        reports.sort_by_key(|s| s.worker_id);
        assert!(reports.iter().all(|s| s.processing_target == native::Time(0)));
        let reading =
            Arc::new(WorkerReading { started_at: Duration::from_millis(2), completed_at: Duration::from_millis(5), workers: reports });
        let mut view = WorkerView::default();
        view.set_reading(reading);
        let text = draw(&mut view, 160, 24);
        assert!(text.contains("128 snapshots"));
        assert!(text.contains(&format!("{workers} workers")));
        let after = runtime.request_worker_status().unwrap();
        for _ in 0..workers {
            assert_eq!(after.recv_timeout(Duration::from_secs(2)).unwrap().processing_target, native::Time(0));
        }
        assert_eq!(after.recv_timeout(Duration::from_secs(2)), Err(crossbeam_channel::RecvTimeoutError::Disconnected));
        runtime.shutdown();
    }
}

#[test]
fn application_totals_rates_and_unavailable_baselines_are_explicit() {
    use contime_monitor::WorkerCounters;
    let mut first = reading(1);
    Arc::get_mut(&mut first).unwrap().workers[0].counters =
        Some(WorkerCounters { inserted_applications: 100, replay_applications: Some(120), saturated: false });
    let mut second = reading(1);
    let next = Arc::get_mut(&mut second).unwrap();
    next.completed_at = Duration::from_millis(1005);
    next.started_at = next.completed_at;
    next.workers[0].counters = Some(WorkerCounters { inserted_applications: 300, replay_applications: Some(420), saturated: false });
    let mut view = WorkerView::default();
    view.set_reading(first);
    let baseline = compact(draw(&mut view, 240, 24));
    assert!(baseline.contains("Total inserted: 100"));
    assert!(baseline.contains("Avg inserted/sec: -"));
    view.set_reading(second);
    let text = compact(draw(&mut view, 240, 24));
    for expected in ["Total inserted: 300", "Total replayed: 420", "Avg inserted/sec: 200", "Avg replayed/sec: 300"] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    let mut off = WorkerView::<u64>::default();
    off.set_reading(reading(1));
    assert!(draw(&mut off, 160, 24).contains("Counters unavailable"));
}

#[test]
fn unknown_replay_and_saturation_are_not_presented_as_exact_measurements() {
    use contime_monitor::WorkerCounters;
    let mut unknown = reading(1);
    Arc::get_mut(&mut unknown).unwrap().workers[0].counters =
        Some(WorkerCounters { inserted_applications: 100, replay_applications: None, saturated: false });
    let mut view = WorkerView::default();
    view.set_reading(unknown);
    let text = compact(draw(&mut view, 240, 24));
    assert!(text.contains("Total replayed: -"));
    assert!(text.contains("Avg replayed/sec: -"));
    let mut saturated = reading(1);
    Arc::get_mut(&mut saturated).unwrap().workers[0].counters =
        Some(WorkerCounters { inserted_applications: u64::MAX, replay_applications: Some(u64::MAX), saturated: true });
    view.set_reading(saturated);
    let text = compact(draw(&mut view, 240, 24));
    assert!(text.contains("SATURATED (lower bounds)"));
    assert!(text.contains("Avg inserted/sec: -"));
}

#[test]
fn digit_changes_keep_summary_labels_and_table_units_in_fixed_columns() {
    use contime_monitor::WorkerCounters;
    fn sample(at: u64, inserted: u64, gauges: usize) -> Arc<WorkerReading<u64>> {
        let mut data = reading(1);
        let data_mut = Arc::get_mut(&mut data).unwrap();
        data_mut.started_at = Duration::from_millis(at);
        data_mut.completed_at = Duration::from_millis(at);
        let worker = &mut data_mut.workers[0];
        worker.history_stores = gauges;
        worker.pending_histories = gauges;
        worker.due_histories = gauges;
        worker.counters = Some(WorkerCounters { inserted_applications: inserted, replay_applications: Some(inserted), saturated: false });
        data
    }
    for width in [80, 160, 240] {
        let mut view = WorkerView::default();
        view.set_reading(sample(0, 100_007, 9));
        view.set_reading(sample(1000, 110_006, 999));
        let low = draw(&mut view, width, 24);
        view.set_reading(sample(2000, 120_006, 1000));
        let high = draw(&mut view, width, 24);
        for label in ["snapshots", "pending", "due", "Collection:", "Total replayed:", "Avg replayed/sec:"] {
            let position = |text: &str| {
                let line = text.lines().find(|line| line.contains(label)).unwrap();
                line.find(label).unwrap()
            };
            assert_eq!(position(&low), position(&high), "{label} shifted at width {width}");
        }
        if width >= 120 {
            let values = |text: &str, value: &str| {
                let lines = text.lines().collect::<Vec<_>>();
                let header = lines.iter().position(|line| line.contains("Worker") && line.contains("Snapshots")).unwrap();
                // Compare character-cell positions, not byte positions of the border.
                lines[header + 1].replace('│', "|").match_indices(value).map(|(i, _)| i + value.len()).collect::<Vec<_>>()
            };
            assert_eq!(values(&low, "9,999"), values(&high, "10,000"), "rate units moved inside table columns");
        }
    }
}

#[test]
fn compact_worker_table_shows_distinct_worker_rates_alongside_global_rates() {
    use contime_monitor::WorkerCounters;
    let make = |at: u64, inserted: [u64; 2], replay: [u64; 2]| {
        let mut result = reading(2);
        let data = Arc::get_mut(&mut result).unwrap();
        data.started_at = Duration::from_millis(at);
        data.completed_at = Duration::from_millis(at);
        for (index, worker) in data.workers.iter_mut().enumerate() {
            worker.counters =
                Some(WorkerCounters { inserted_applications: inserted[index], replay_applications: Some(replay[index]), saturated: false });
        }
        result
    };
    let mut view = WorkerView::default();
    view.set_reading(make(0, [100, 200], [200, 300]));
    view.set_reading(make(1000, [1_300_100, 700_200], [1_300_200, 700_300]));
    for width in [120, 150, 180] {
        let text = draw(&mut view, width, 12);
        let lines: Vec<_> = text.lines().collect();
        let header = lines.iter().position(|line| line.contains("Worker") && line.contains("Snapshots")).unwrap();
        assert!(lines[header].contains("Avg inserted/sec"), "missing worker insertion rate at width {width}: {text}");
        assert!(lines[header].contains("Avg replayed/sec"), "missing worker replay rate at width {width}: {text}");
        assert!(lines[header + 1].contains("1,300,000"), "missing worker 0 throughput: {text}");
        assert!(lines[header + 2].contains("700,000"), "missing worker 1 throughput: {text}");
        let summary = lines.iter().find(|line| line.contains("Avg inserted/sec:")).unwrap();
        assert!(summary.contains("2,000,000"), "missing global throughput: {text}");
        if width >= 80 {
            assert!(lines[header].contains("Total inserted"));
            assert!(lines[header + 1].contains("1,300,100"));
            assert!(lines[header + 2].contains("700,200"));
        }
    }
}

#[test]
fn cumulative_columns_are_explicitly_labelled_as_totals() {
    use contime_monitor::WorkerCounters;
    let mut data = reading(1);
    Arc::get_mut(&mut data).unwrap().workers[0].counters =
        Some(WorkerCounters { inserted_applications: 12_345_678, replay_applications: Some(23_456_789), saturated: false });
    let mut view = WorkerView::default();
    view.set_reading(data);
    let text = draw(&mut view, 80, 12);
    assert!(text.contains("Total inserted"), "{text}");
    assert!(text.contains("Total replay"), "{text}");
    assert!(text.contains("12,345,678"), "{text}");
    assert!(text.contains("23,456,789"), "{text}");
}

#[test]
fn narrow_view_stacks_totals_and_rates_instead_of_hiding_cumulative_counts() {
    use contime_monitor::WorkerCounters;
    let make = |at, values: [u64; 2]| {
        let mut data = reading(2);
        let r = Arc::get_mut(&mut data).unwrap();
        r.started_at = Duration::from_millis(at);
        r.completed_at = Duration::from_millis(at);
        for (worker, value) in r.workers.iter_mut().zip(values) {
            worker.counters = Some(WorkerCounters { inserted_applications: value, replay_applications: Some(value), saturated: false });
        }
        data
    };
    let mut view = WorkerView::default();
    view.set_reading(make(0, [100, 200]));
    view.set_reading(make(1000, [1_300_100, 700_200]));
    let first = draw(&mut view, 48, 12);
    assert!(first.contains("Total"), "{first}");
    assert!(first.contains("Avg/sec"), "{first}");
    assert!(first.contains("1,300,100"), "{first}");
    assert!(first.contains("1,300,000"), "{first}");
    view.scroll(Scroll::Down);
    let second = draw(&mut view, 48, 12);
    assert!(second.contains("700,200"), "{second}");
    assert!(second.contains("700,000"), "{second}");
    assert!(second.contains("2-2 of 2"), "{second}");
}

#[test]
fn worker_table_shows_only_cumulative_average_rates() {
    use contime_monitor::WorkerCounters;
    let make = |at, values: [u64; 2]| {
        let mut data = reading(2);
        let r = Arc::get_mut(&mut data).unwrap();
        r.started_at = Duration::from_secs(at);
        r.completed_at = Duration::from_secs(at);
        for (worker, value) in r.workers.iter_mut().zip(values) {
            worker.counters = Some(WorkerCounters { inserted_applications: value, replay_applications: Some(value * 2), saturated: false });
        }
        data
    };
    for (width, height) in [48, 80, 120, 160, 240].into_iter().flat_map(|width| [12, 13, 14, 15, 24].map(|height| (width, height))) {
        let mut view = WorkerView::default();
        // Nonzero initial counts must not inflate the monitored average.
        view.set_reading(make(10, [100, 200]));
        view.set_reading(make(11, [100_100, 200_200]));
        view.set_reading(make(12, [400_100, 800_200]));
        let text = draw(&mut view, width, height);
        for expected in ["200,000", "400,000"] {
            assert!(text.contains(expected), "missing worker 0 average {expected} at {width}: {text}");
        }
        if width < 120 {
            assert!(text.contains("Avg/sec"), "{text}");
            view.scroll(Scroll::End);
        } else {
            assert!(text.contains("Avg inserted/sec"), "{text}");
            assert!(text.contains("Avg replayed/sec"), "{text}");
        }
        let text = draw(&mut view, width, height);
        for expected in ["400,000", "800,000"] {
            assert!(text.contains(expected), "missing worker 1 average {expected} at {width}: {text}");
        }
        let table = text.split("┌ Workers").nth(1).unwrap();
        assert!(!table.contains("300,000"), "latest worker rate still shown: {text}");
        assert!(!table.contains("1,200,000"), "latest worker replay rate still shown: {text}");
        assert!(!table.contains("per sec"), "latest-rate row still shown: {text}");
        // Counter resets must produce an unavailable average, not a false rate.
        view.set_reading(make(13, [0, 0]));
        let text = draw(&mut view, width, height);
        assert!(!text.contains("200,000"), "stale average after reset: {text}");
    }
}
