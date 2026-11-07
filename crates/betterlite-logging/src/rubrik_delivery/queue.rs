use std::sync::PoisonError;
use std::sync::atomic::Ordering;

use betterlite_models::decisions::Json;

use super::config::DROP_WARNING_INTERVAL;
use super::error::{RubrikError, Service};
use super::logger::RubrikLogger;
use super::transport::{ClientTier, JsonPost};

pub fn push_payload(logger: &RubrikLogger, payload: Json) {
    let mut queue = logger.queue.lock().unwrap_or_else(PoisonError::into_inner);
    queue.push_back(payload);
    drop(queue);
    enforce_max_queue_size(logger);
}

pub async fn append_and_maybe_flush(logger: &RubrikLogger, payload: Json) {
    push_payload(logger, payload);
    if logger.state().length >= logger.settings.batch_size {
        flush_queue(logger).await;
    }
}

#[must_use]
pub fn queue_snapshot(logger: &RubrikLogger) -> Vec<Json> {
    let queue = logger.queue.lock().unwrap_or_else(PoisonError::into_inner);
    let mut snapshot = Vec::with_capacity(queue.len());
    snapshot.extend(queue.iter().cloned());
    snapshot
}

pub async fn flush_queue(logger: &RubrikLogger) {
    let _guard = logger.flush_lock.lock().await;
    let snapshot = queue_snapshot(logger);
    if snapshot.is_empty() {
        return;
    }
    tracing::debug!(events = snapshot.len(), "Rubrik: flushing batch");
    if log_batch_to_rubrik(logger, &snapshot).await.is_err() {
        return;
    }
    let mut queue = logger.queue.lock().unwrap_or_else(PoisonError::into_inner);
    let drain_count = snapshot.len().min(queue.len());
    queue.drain(..drain_count);
    drop(queue);
    let now = logger.clock.now_unix();
    let mut last_flush_time = logger
        .last_flush_time
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    *last_flush_time = now;
}

pub async fn async_send_batch(logger: &RubrikLogger) -> Result<(), RubrikError> {
    let payloads = queue_snapshot(logger);
    if payloads.is_empty() {
        return Ok(());
    }
    log_batch_to_rubrik(logger, &payloads).await
}

fn enforce_max_queue_size(logger: &RubrikLogger) {
    let overflow = logger
        .state()
        .length
        .saturating_sub(logger.settings.max_queue_size);
    if overflow == 0 {
        return;
    }
    let mut queue = logger.queue.lock().unwrap_or_else(PoisonError::into_inner);
    let drain_count = overflow.min(queue.len());
    queue.drain(..drain_count);
    drop(queue);
    let overflow_count = u64::try_from(overflow).unwrap_or(u64::MAX);
    let dropped = logger
        .dropped_since_warning
        .fetch_add(overflow_count, Ordering::SeqCst)
        .saturating_add(overflow_count);
    let now = logger.clock.now_unix();
    if now.saturating_sub(logger.state().last_drop_warning_time) >= DROP_WARNING_INTERVAL {
        tracing::warn!(
            max_queue_size = logger.settings.max_queue_size,
            dropped,
            "Rubrik: log queue exceeded max_queue_size; dropped oldest events since the last warning"
        );
        logger.dropped_since_warning.store(0, Ordering::SeqCst);
        let mut last_warning = logger
            .last_drop_warning_time
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *last_warning = now;
    }
}

async fn log_batch_to_rubrik(logger: &RubrikLogger, payloads: &[Json]) -> Result<(), RubrikError> {
    let body = serde_json::to_vec(payloads).map_err(|source| RubrikError::Serialize {
        service: Service::Logging,
        source,
    })?;
    let request = JsonPost {
        tier: ClientTier::Logging,
        url: logger.settings.logging_endpoint.clone(),
        headers: logger.headers.clone(),
        body,
        timeout: logger.settings.logging_timeout,
    };
    let response = logger
        .transport
        .post_json(request)
        .await
        .map_err(|source| {
            tracing::error!(error = %source, "Rubrik Layer Error");
            RubrikError::Transport {
                service: Service::Logging,
                source,
            }
        })?;
    if response.status >= 400 {
        let text = str::from_utf8(&response.body).unwrap_or("<non-UTF-8 body>");
        tracing::error!(status = response.status, body = text, "Rubrik HTTP Error");
        return Err(RubrikError::HttpStatus {
            service: Service::Logging,
            status: response.status,
        });
    }
    Ok(())
}
