//! Factorial comparison: builder routing and typed application can be measured independently.
use super::*;
use contime::{
    checkpoints::{Event, Snapshot},
    module::EventSource,
    Input,
};

struct ProbeInput<const ROUTING: bool>(AppEventLanes);
impl<const R: bool> Event for ProbeInput<R> {
    type Time = i64;
    fn time(&self) -> i64 {
        self.0.time()
    }
}
impl<const R: bool> EventSource for ProbeInput<R> {
    fn event_ref(&self) -> EventRef<'_> {
        self.0.event_ref()
    }
    fn get<E: std::any::Any>(&self) -> Option<&E> {
        self.0.get::<E>()
    }
    fn contains_event_type(id: std::any::TypeId) -> bool {
        AppEventLanes::contains_event_type(id)
    }
}
impl<const R: bool> Input for ProbeInput<R> {
    fn event_id(&self) -> u128 {
        Input::event_id(&self.0)
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        if R {
            self.0.snapshot_ids(emit)
        } else {
            emit(match &self.0 {
                AppEventLanes::Added(e) => e.owner,
                AppEventLanes::Bonus(e) => e.owner,
            })
        }
    }
}
#[derive(Clone)]
struct ProbeState<const APPLY: bool>(Total);
impl<const A: bool> Snapshot for ProbeState<A> {
    type Time = i64;
    fn time(&self) -> &i64 {
        Snapshot::time(&self.0)
    }
    fn set_time(&mut self, t: i64) {
        self.0.set_time(t);
    }
}
impl<const R: bool, const A: bool> ApplyEvents<ProbeInput<R>> for ProbeState<A> {
    fn create(id: u128, _: &ProbeInput<R>) -> Self {
        Self(Total { owner: id, ..Total::default() })
    }
    #[inline(always)]
    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, i64, ProbeInput<R>>) {
        if A {
            contime::module::apply_batch(&mut self.0, batch);
        } else {
            for event in batch.events {
                self.0.value += match &event.0 {
                    AppEventLanes::Added(e) => e.value,
                    AppEventLanes::Bonus(e) => e.value,
                };
            }
        }
    }
}
fn sample<const R: bool, const A: bool>(count: usize, workers: usize) -> Duration {
    const SNAPSHOTS: usize = 256;
    assert_eq!(std::mem::size_of::<ProbeInput<R>>(), std::mem::size_of::<AppEventLanes>());
    assert_eq!(std::mem::size_of::<ProbeState<A>>(), std::mem::size_of::<AppSnapshotLanes>());
    let inputs = events(count, SNAPSHOTS).into_iter().map(ProbeInput::<R>).collect::<Vec<_>>();
    let contime = contime::RawConTime::<ProbeInput<R>, ProbeState<A>, ()>::start(config(workers), ()).unwrap();
    let start = Instant::now();
    let mut inputs = inputs.into_iter();
    loop {
        let batch = inputs.by_ref().take(1024).collect::<Vec<_>>();
        if batch.is_empty() {
            break;
        }
        contime.apply(batch).unwrap();
    }
    contime.advance_to((count / SNAPSHOTS) as i64).unwrap();
    contime.wait_until_idle(Duration::from_secs(10)).unwrap();
    let elapsed = start.elapsed();
    let states = contime.query_at((count / SNAPSHOTS) as i64, 0..SNAPSHOTS as u128).unwrap();
    assert_eq!(states.len(), SNAPSHOTS);
    for state in &states {
        assert_eq!(state.0.value, (count / SNAPSHOTS) as u64 * (1 + state.0.owner as u64 % 2));
    }
    assert_eq!(states.iter().map(|s| s.0.value).sum::<u64>(), (count / 2 * 3) as u64);
    assert!(contime.errors().is_empty());
    contime.shutdown();
    elapsed
}
fn config(workers: usize) -> ConTimeConfig<i64> {
    ConTimeConfig {
        router_count: 1,
        worker_count: workers,
        placement: Placement::default(),
        pruning_interval: Duration::from_secs(60),
        history_retention: i64::MAX,
        checkpoints: CheckpointConfig { interval: 128 },
    }
}
fn generated_sample(count: usize, workers: usize) -> Duration {
    let inputs = events(count, 256);
    let contime = ConTime::<App>::start(config(workers)).unwrap();
    let start = Instant::now();
    let mut inputs = inputs.into_iter();
    loop {
        let batch = inputs.by_ref().take(1024).collect::<Vec<_>>();
        if batch.is_empty() {
            break;
        }
        contime.apply(batch).unwrap();
    }
    contime.advance_to((count / 256) as i64).unwrap();
    contime.wait_until_idle(Duration::from_secs(10)).unwrap();
    let elapsed = start.elapsed();
    let states = contime.query_at((count / 256) as i64, 0..256).unwrap();
    assert_eq!(states.len(), 256);
    for state in &states {
        let AppSnapshotLanes::Total(s) = &**state;
        assert_eq!(s.value, (count / 256) as u64 * (1 + s.owner as u64 % 2));
    }
    let sum: u64 = states
        .iter()
        .map(|s| {
            let AppSnapshotLanes::Total(s) = &**s;
            s.value
        })
        .sum();
    assert_eq!(sum, (count / 2 * 3) as u64);
    assert!(contime.errors().is_empty());
    contime.shutdown();
    elapsed
}
fn runtime<const R: bool, const A: bool>(c: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>, name: &str) {
    const COUNT: usize = 65536;
    c.throughput(Throughput::Elements(COUNT as u64));
    c.bench_function(name, |b| b.iter_custom(|n| (0..n).map(|_| sample::<R, A>(COUNT, 1)).sum()));
}
// Enumerate every order, then shuffle with a fixed seed. This balances both
// each case's position and every within-round predecessor/successor pair.
fn schedules() -> Vec<[usize; 5]> {
    fn enumerate(order: &mut [usize; 5], index: usize, result: &mut Vec<[usize; 5]>) {
        if index == order.len() {
            result.push(*order);
            return;
        }
        for next in index..order.len() {
            order.swap(index, next);
            enumerate(order, index + 1, result);
            order.swap(index, next);
        }
    }
    let mut orders = Vec::new();
    enumerate(&mut [0, 1, 2, 3, 4], 0, &mut orders);
    assert_eq!(orders.len(), 120);
    let mut positions = [[0; 5]; 5];
    let mut neighbors = [[0; 5]; 5];
    for order in &orders {
        for (position, &case) in order.iter().enumerate() {
            positions[case][position] += 1;
        }
        for pair in order.windows(2) {
            neighbors[pair[0]][pair[1]] += 1;
        }
    }
    assert!(positions.iter().flatten().all(|&n| n == 24));
    for (from, row) in neighbors.iter().enumerate() {
        for (to, &count) in row.iter().enumerate() {
            assert_eq!(count, if from == to { 0 } else { 24 });
        }
    }
    fastrand::Rng::with_seed(0x434f_4e54_494d_4501).shuffle(&mut orders);
    orders
}
pub(super) fn paired() {
    const COUNT: usize = 65536;
    type Sample = fn(usize, usize) -> Duration;
    let cases: [(&str, Sample); 5] = [
        ("direct_route_direct_apply", sample::<false, false>),
        ("builder_route_direct_apply", sample::<true, false>),
        ("direct_route_typed_apply", sample::<false, true>),
        ("builder_route_typed_apply", sample::<true, true>),
        ("generated_module", generated_sample),
    ];
    let orders = schedules();
    let rounds = orders.len();
    App::validate();
    for workers in [1, 4] {
        for (_, run) in cases {
            run(COUNT, workers);
        }
        let mut times: [Vec<f64>; 5] = std::array::from_fn(|_| Vec::new());
        for order in &orders {
            for &index in order {
                times[index].push(cases[index].1(COUNT, workers).as_secs_f64());
            }
        }
        println!("workers={workers}, events/sample={COUNT}, rounds={rounds}");
        for (i, (name, _)) in cases.iter().enumerate() {
            println!("{name}: {:.4} M events/s", COUNT as f64 * rounds as f64 / times[i].iter().sum::<f64>() / 1e6);
            if i > 0 {
                let mut ratios = times[i].iter().zip(&times[0]).map(|(t, base)| 100.0 * (t / base - 1.0)).collect::<Vec<_>>();
                ratios.sort_by(f64::total_cmp);
                println!(
                    "  paired time change vs direct: median {:+.2}%, middle half [{:+.2}%,{:+.2}%]",
                    ratios[rounds / 2],
                    ratios[rounds / 4],
                    ratios[3 * rounds / 4]
                );
            }
        }
    }
}
pub(super) fn benchmarks(c: &mut Criterion) {
    App::validate();
    let inputs = events(256, 256);
    let mut g = c.benchmark_group("module_application/routing");
    g.sample_size(30);
    g.warm_up_time(Duration::from_millis(500));
    g.measurement_time(Duration::from_secs(1));
    g.throughput(Throughput::Elements(inputs.len() as u64));
    g.bench_function("builder", |b| {
        b.iter(|| {
            for e in black_box(&inputs) {
                e.snapshot_ids(&mut |id| {
                    black_box(id);
                });
            }
        })
    });
    g.bench_function("direct", |b| {
        b.iter(|| {
            for e in black_box(&inputs) {
                black_box(match e {
                    AppEventLanes::Added(e) => e.owner,
                    AppEventLanes::Bonus(e) => e.owner,
                });
            }
        })
    });
    g.finish();
    let mut g = c.benchmark_group("module_application/isolation");
    g.sample_size(20);
    g.warm_up_time(Duration::from_secs(1));
    g.measurement_time(Duration::from_secs(2));
    runtime::<false, false>(&mut g, "direct_route_direct_apply");
    runtime::<true, false>(&mut g, "builder_route_direct_apply");
    runtime::<false, true>(&mut g, "direct_route_typed_apply");
    runtime::<true, true>(&mut g, "builder_route_typed_apply");
    g.finish();
}
