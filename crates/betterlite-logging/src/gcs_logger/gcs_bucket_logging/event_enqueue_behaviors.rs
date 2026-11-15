use std::fmt::Debug;
use std::sync::Arc;
use std::sync::PoisonError;

use betterlite_models::decisions::Json;

use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_mock::client::GcsHttpClient;

use super::flush_behaviors::flush_queue;
use super::{GcsBucketLogger, QueuedItem};

// LiteLLM flushes a full queue once to make room, then puts the item; asyncio's maxsize 0 means unbounded
pub async fn enqueue<Auth, Http>(logger: &GcsBucketLogger<Auth, Http>, event: &LoggerEvent)
where
    Auth: VertexAuth + Clone + Debug,
    Http: GcsHttpClient + Debug,
{
    let capacity = logger.settings.queue_capacity;
    let queued = logger
        .queue
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .len();
    if capacity > 0 && queued >= capacity {
        flush_queue(logger).await;
    }
    let item = queued_item(event);
    let mut queue = logger.queue.lock().unwrap_or_else(PoisonError::into_inner);
    if capacity > 0 && queue.len() >= capacity {
        tracing::error!(
            capacity,
            "GCS Bucket log queue is still full after a flush; the log is dropped"
        );
        return;
    }
    queue.push_back(item);
}

fn queued_item(event: &LoggerEvent) -> QueuedItem {
    QueuedItem {
        payload: Arc::clone(&event.payload),
        dynamic_params: Arc::clone(&event.dynamic_params),
        metadata: Arc::clone(&event.litellm_metadata),
        response_id: response_id(event.payload.as_ref()),
    }
}

// response_obj["id"] reaches the logger as response.id on the standard logging payload
fn response_id(payload: &StandardLoggingPayload) -> String {
    let Some(Json::Object(fields)) = payload.response.as_ref() else {
        return String::new();
    };
    match fields.get("id") {
        Some(Json::Text(id)) => id.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => String::new(),
    }
}

mod tests;
