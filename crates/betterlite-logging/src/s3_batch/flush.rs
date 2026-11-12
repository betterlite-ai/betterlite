use crate::private_json::error::PrivateJsonError;
use crate::s3_batch::ndjson_batch_file::{
    batch_file_elements, batch_file_mode_active, batch_stamp,
};
use crate::s3_batch::queue::BatchQueue;
use crate::s3_batch::upload_bounded::MAX_WAITERS;
use crate::s3_batch::{Element, S3BatchUploadError};
use crate::s3_transport::signing::Clock;
use crate::s3_transport::transport::UploadOutcome;
use futures_util::future::join_all;
use std::fmt::Debug;
use std::future::Future;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::time::{Duration, Instant};
pub trait MonotonicClock: Send + Sync + Debug {
    fn seconds(&self) -> f64;
}
#[derive(Debug)]
pub struct SystemMonotonic {
    start: Instant,
}
impl Default for SystemMonotonic {
    fn default() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}
impl MonotonicClock for SystemMonotonic {
    fn seconds(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
}
#[derive(Debug, Clone)]
pub struct FlushSettings {
    pub max_retry_age: Option<Duration>,
    pub max_queue_size: usize,
    pub batch_file_upload: bool,
    pub cold_storage_logger: Option<String>,
    pub max_concurrent_uploads: usize,
}
pub struct FlushContext<'flush> {
    pub queue: &'flush BatchQueue,
    pub settings: &'flush FlushSettings,
    pub clock: &'flush dyn Clock,
    pub monotonic: &'flush dyn MonotonicClock,
}
pub trait UploadOperation: Send + Sync {
    fn upload<'operation>(
        &'operation self,
        element: &'operation Element,
    ) -> Pin<Box<dyn Future<Output = UploadOutcome> + Send + 'operation>>;
}
#[derive(Debug, thiserror::Error)]
pub enum FlushError {
    #[error(transparent)]
    Upload(#[from] S3BatchUploadError),
    #[error("an S3 batch file could not be serialized")]
    Serialize(#[source] PrivateJsonError),
}
pub async fn send_batch(
    context: &FlushContext<'_>,
    upload: &dyn UploadOperation,
) -> Result<usize, FlushError> {
    let snapshot = context.queue.snapshot();
    let batch_len = snapshot.entries.len();
    if batch_len == 0 {
        return Ok(0);
    }
    let grouped = batch_file_mode_active(
        context.settings.batch_file_upload,
        context.settings.cold_storage_logger.as_deref(),
    );
    let uploads = if grouped {
        batch_file_elements(&snapshot.entries, &batch_stamp(context.clock))
            .map_err(FlushError::Serialize)?
    } else {
        snapshot.entries
    };
    let order = upload_order(uploads.len(), batch_len, snapshot.requeued_count);
    let outcomes = run_uploads(
        &uploads,
        &order,
        context.settings.max_concurrent_uploads,
        upload,
    )
    .await;
    reconcile(context, batch_len, &uploads, &outcomes)?;
    Ok(batch_len)
}
fn upload_order(uploads: usize, batch: usize, requeued: usize) -> Vec<usize> {
    let stale = if uploads == batch {
        requeued.min(uploads)
    } else {
        0
    };
    (stale..uploads).chain(0..stale).collect()
}
async fn run_uploads(
    queue: &[Element],
    order: &[usize],
    window: usize,
    upload: &dyn UploadOperation,
) -> Vec<UploadOutcome> {
    let mut outcomes = vec![UploadOutcome::Retry; queue.len()];
    let chunk = window.saturating_add(MAX_WAITERS).max(1);
    for chunk_order in order.chunks(chunk) {
        let pending = chunk_order
            .iter()
            .filter_map(|index| queue.get(*index))
            .map(|element| upload.upload(element));
        let settled = join_all(pending).await;
        for (index, outcome) in chunk_order.iter().zip(settled) {
            if let Some(slot) = outcomes.get_mut(*index) {
                *slot = outcome;
            }
        }
    }
    outcomes
}
fn reconcile(
    context: &FlushContext<'_>,
    batch_len: usize,
    uploads: &[Element],
    outcomes: &[UploadOutcome],
) -> Result<(), FlushError> {
    let delivered = outcomes
        .iter()
        .filter(|outcome| **outcome == UploadOutcome::Delivered)
        .count();
    let bucket_wide = delivered == 0;
    let failed: Vec<(&Element, UploadOutcome)> = uploads
        .iter()
        .zip(outcomes)
        .filter(|(_element, outcome)| **outcome != UploadOutcome::Delivered)
        .map(|(element, outcome)| (element, *outcome))
        .collect();
    let requeued = survivors(context, &failed, bucket_wide);
    if let Some(dropped) = NonZeroUsize::new(failed.len().saturating_sub(requeued.len())) {
        tracing::warn!(dropped = dropped.get(), max_retry_age = ?context.settings.max_retry_age, "s3 logging: uploads dropped (terminal, stale or over the retry age)");
    }
    if requeued.is_empty() {
        return Ok(());
    }
    if let Some(trimmed) =
        NonZeroUsize::new(context.queue.restore_after_failure(batch_len, requeued))
    {
        tracing::warn!(
            max_queue_size = context.settings.max_queue_size,
            dropped = trimmed.get(),
            "s3 logging: queue exceeded max_queue_size; dropped the oldest events"
        );
    }
    Err(FlushError::Upload(S3BatchUploadError {
        failed: failed.len(),
        total: uploads.len(),
    }))
}
fn survivors(
    context: &FlushContext<'_>,
    failed: &[(&Element, UploadOutcome)],
    bucket_wide: bool,
) -> Vec<Element> {
    let now = context.monotonic.seconds();
    let mut requeued = Vec::new();
    for (element, outcome) in failed {
        if bucket_wide {
            requeued.push((*element).clone());
            continue;
        }
        if *outcome == UploadOutcome::Dropped {
            continue;
        }
        if let Some(kept) = survivor(element, context.settings.max_retry_age, now) {
            requeued.push(kept);
        }
    }
    requeued
}

fn survivor(element: &Element, max_retry_age: Option<Duration>, now: f64) -> Option<Element> {
    if let Some(limit) = max_retry_age
        && let Some(since) = element.retrying_since
        && since.mul_add(-1.0, now) > limit.as_secs_f64()
    {
        return None;
    }
    let mut kept = element.clone();
    if kept.retrying_since.is_none() && max_retry_age.is_some() {
        kept.retrying_since = Some(now);
    }
    Some(kept)
}
mod tests;
