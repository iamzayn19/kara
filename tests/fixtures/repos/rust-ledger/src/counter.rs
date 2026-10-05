use std::sync::atomic::{AtomicU64, Ordering};

/// Counts processed requests from many threads.
#[derive(Debug, Default)]
pub struct RequestCounter {
    count: AtomicU64,
}

impl RequestCounter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self) {
        let current = self.count.load(Ordering::SeqCst);
        std::thread::yield_now();
        self.count.store(current + 1, Ordering::SeqCst);
    }

    pub fn get(&self) -> u64 {
        self.count.load(Ordering::SeqCst)
    }
}
