use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::core::PointFiveCore;
use super::send;

pub(crate) async fn flush_queue(core: &PointFiveCore, skip_if_flushing: bool) {
    if core.queue.is_empty() {
        ping(core).await;
        return;
    }
    if skip_if_flushing && core.flushing.load(Ordering::SeqCst) {
        return;
    }
    flush_locked(core).await;
}

// periodic_flush: report in straight away, then flush on the interval the inherited loop uses
pub(crate) async fn periodic_flush(
    core: &PointFiveCore,
    interval: Duration,
    shutdown: CancellationToken,
) {
    flush_queue(core, true).await;
    loop {
        if timeout(interval, shutdown.cancelled()).await.is_ok() {
            return;
        }
        flush_queue(core, false).await;
    }
}

// _ping: report liveness, never failing the flush over it
pub(crate) async fn ping(core: &PointFiveCore) {
    let client = match core.upload_client() {
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
async fn flush_locked(core: &PointFiveCore) {
    let _flush = core.flush_lock.lock().await;
    core.flushing.store(true, Ordering::SeqCst);
    let snapshot = core.queue.len();
    if snapshot != 0 {
        match send::send_batch(core).await {
            Err(error) => {
                tracing::error!(
                    %error,
                    records = snapshot,
                    "pointfive: the batch send failed; the unsent records stay queued"
                );
                core.trim_queue();
            }
            Ok(()) => core.queue.remove_prefix(snapshot),
        }
    }
    core.flushing.store(false, Ordering::SeqCst);
}
