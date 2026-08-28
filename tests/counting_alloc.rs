//! Allocation-count pin for `parse_v2`, in its own binary for the `#[global_allocator]`.

#![allow(missing_docs, unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use wal2json_events::parse_v2;

static MEASURING: AtomicBool = AtomicBool::new(false);
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

struct CountingAlloc;

// SAFETY: all alloc and dealloc calls are forwarded unchanged to the System allocator.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if MEASURING.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: forwarded unchanged to System.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded unchanged to System.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static COUNTING_ALLOC: CountingAlloc = CountingAlloc;

fn start_measuring() {
    ALLOC_COUNT.store(0, Ordering::Relaxed);
    // Relaxed: parse_v2 runs on this thread, after the store.
    MEASURING.store(true, Ordering::Relaxed);
}

fn stop_measuring() -> usize {
    MEASURING.store(false, Ordering::Relaxed);
    ALLOC_COUNT.load(Ordering::Relaxed)
}

const FIXTURE: &str = r#"{"action":"I","schema":"public","table":"users","columns":[{"name":"id","type":"integer","value":1},{"name":"name","type":"text","value":"Alice"}]}"#;

#[test]
fn parse_v2_allocation_count() {
    // schema, table, Vec<Column> store, col-0 name, col-0 type, col-1 name, col-1 type, "Alice" String.
    // Review before raising: an intermediate Value parse roughly doubles it, and letting serde
    // deserialize the internally tagged enum directly costs 12, since it buffers the map.
    #[cfg(not(feature = "arbitrary_precision"))]
    const EXPECTED: usize = 8;
    // arbitrary_precision keeps every number as a String, one allocation each.
    #[cfg(feature = "arbitrary_precision")]
    const EXPECTED: usize = 10;

    start_measuring();
    let _msg = parse_v2(FIXTURE).unwrap();
    let observed = stop_measuring();

    assert_eq!(
        observed, EXPECTED,
        "allocation count changed to {observed}: review and update EXPECTED if intentional"
    );
}
