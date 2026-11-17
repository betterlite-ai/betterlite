use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::core::PointFiveCore;

impl PointFiveCore {
    pub async fn flush_queue(&self, skip_if_flushing: bool) {
        if self.queue.is_empty() {
            self.ping().await;
            return;
        }
        if skip_if_flushing && self.flushing.load(Ordering::SeqCst) {
            return;
        }
        self.flush_locked().await;
    }

    // periodic_flush: report in straight away, then flush on the interval the inherited loop uses
    pub async fn periodic_flush(&self, interval: Duration, shutdown: CancellationToken) {
        self.flush_queue(true).await;
        loop {
            if timeout(interval, shutdown.cancelled()).await.is_ok() {
                return;
            }
            self.flush_queue(false).await;
        }
    }

    // _ping: report liveness, never failing the flush over it
    pub async fn ping(&self) {
        let client = match self.upload_client() {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!(%error, "pointfive: liveness ping skipped");
                return;
            }
        };
        if let Err(failure) = client.ping().await {
            tracing::warn!(detail = %failure.detail, "pointfive: liveness ping failed");
        }
    }

    // the flag is held under the flush lock, so a queued flush keeps it set while it sends
    async fn flush_locked(&self) {
        let _flush = self.flush_lock.lock().await;
        self.flushing.store(true, Ordering::SeqCst);
        let snapshot = self.queue.len();
        if snapshot != 0 {
            match self.send_batch().await {
                Err(error) => {
                    tracing::error!(
                        %error,
                        records = snapshot,
                        "pointfive: the batch send failed; the unsent records stay queued"
                    );
                    self.trim_queue();
                }
                Ok(()) => self.queue.remove_prefix(snapshot),
            }
        }
        self.flushing.store(false, Ordering::SeqCst);
    }
}
