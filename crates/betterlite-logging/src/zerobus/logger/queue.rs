use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub(super) struct FlushFlag<'flag>(&'flag AtomicBool);

impl<'flag> FlushFlag<'flag> {
    pub(super) fn set(flag: &'flag AtomicBool) -> Self {
        flag.store(true, Ordering::SeqCst);
        Self(flag)
    }
}

impl Drop for FlushFlag<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

pub(super) struct Inflight<'counter>(&'counter AtomicUsize);

impl<'counter> Inflight<'counter> {
    pub(super) fn set(counter: &'counter AtomicUsize, rows: usize) -> Self {
        counter.store(rows, Ordering::SeqCst);
        Self(counter)
    }
}

impl Drop for Inflight<'_> {
    fn drop(&mut self) {
        self.0.store(0, Ordering::SeqCst);
    }
}
