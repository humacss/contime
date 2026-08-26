use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use contime::{RoutePartitionBenchmark, TestEvent, TestInputLanes, TestSnapshotLanes};

struct CountingAllocator;

static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc_zeroed(layout)
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        System.realloc(pointer, layout, new_size)
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        System.dealloc(pointer, layout);
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
fn one_worker_partition_uses_request_level_allocations() {
    let partitioner = RoutePartitionBenchmark::new(1);
    let inputs = (0..1_000).map(|event_id| TestEvent::Positive(7, 10, event_id, 1).into()).collect::<Vec<TestInputLanes>>();
    ALLOCATIONS.store(0, Ordering::Relaxed);
    COUNTING.store(true, Ordering::Relaxed);

    let (affected_workers, routed_events) = partitioner.partition::<TestSnapshotLanes, TestInputLanes, _>(inputs);

    COUNTING.store(false, Ordering::Relaxed);
    let allocations = ALLOCATIONS.load(Ordering::Relaxed);
    println!("one-worker 1,000-event partition allocations: {allocations}");
    assert!(allocations <= 8, "router allocated {allocations} times for one 1,000-event worker batch");
    assert_eq!(affected_workers, 1);
    assert_eq!(routed_events, 1_000);
}
