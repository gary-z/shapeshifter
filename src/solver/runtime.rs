//! Platform plumbing; search choices are shared between native and browser builds.

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use std::time::{Duration, Instant};
#[cfg(target_arch = "wasm32")]
pub(crate) use web_time::{Duration, Instant};

#[cfg(target_arch = "wasm32")]
pub(crate) static CANCELLED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[inline]
pub(crate) fn cancelled() -> bool {
    #[cfg(target_arch = "wasm32")]
    return CANCELLED.load(std::sync::atomic::Ordering::Relaxed) != 0;
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub(crate) fn workers() -> usize {
    #[cfg(not(target_arch = "wasm32"))]
    return std::thread::available_parallelism().map_or(1, usize::from);
    #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
    return rayon::current_num_threads();
    #[cfg(all(target_arch = "wasm32", not(feature = "wasm-threads")))]
    1
}

pub(super) fn for_each_worker<F: Fn(usize) + Sync>(count: usize, work: F) {
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::scope(|scope| {
        for id in 0..count {
            let work = &work;
            scope.spawn(move || work(id));
        }
    });
    #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
    rayon::in_place_scope(|scope| {
        for id in 0..count {
            let work = &work;
            scope.spawn(move |_| work(id));
        }
    });
    #[cfg(all(target_arch = "wasm32", not(feature = "wasm-threads")))]
    for id in 0..count {
        work(id);
    }
}
