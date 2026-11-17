use std::sync::PoisonError;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::Inner;
use super::delivery::ClientDelivery;

struct FlushFlag<'flag>(&'flag AtomicBool);

impl<'flag> FlushFlag<'flag> {
    fn set(flag: &'flag AtomicBool) -> Self {
        flag.store(true, Ordering::SeqCst);
        Self(flag)
    }
}

impl Drop for FlushFlag<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

struct Inflight<'counter>(&'counter AtomicUsize);

impl<'counter> Inflight<'counter> {
    fn set(counter: &'counter AtomicUsize, rows: usize) -> Self {
        counter.store(rows, Ordering::SeqCst);
        Self(counter)
    }
}

impl Drop for Inflight<'_> {
    fn drop(&mut self) {
        self.0.store(0, Ordering::SeqCst);
    }
}

impl Inner {
    // a mid-flush arrival is counted against the rows the delivery holds, so the cap still holds
    pub(super) async fn flush_queue(&self, skip_if_flushing: bool) {
        if skip_if_flushing && self.flushing.load(Ordering::SeqCst) {
            return;
        }
        let _flushing = FlushFlag::set(&self.flushing);
        let pending = {
            let queue = self
                .queue
                .queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            queue.len()
        };
        let _inflight = Inflight::set(&self.inflight, pending);
        self.queue.flush(&ClientDelivery { inner: self }).await;
    }

    pub(super) async fn periodic_flush(&self, shutdown: &CancellationToken) {
        loop {
            if timeout(self.queue.settings.flush_interval, shutdown.cancelled())
                .await
                .is_ok()
            {
                return;
            }
            self.flush_queue(false).await;
        }
    }
}
