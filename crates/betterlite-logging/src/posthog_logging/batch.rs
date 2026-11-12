use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use betterlite_http::user_url::error::UserUrlError;
use http::header::{CONTENT_TYPE, HOST, HeaderMap, HeaderValue, InvalidHeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use subtle::ConstantTimeEq as _;
use thiserror::Error;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::batch::delivery::{BatchCancelled, BatchDelivery};
use crate::callback_logger::event::LoggerEvent;

use super::delivery::{LOGGING_TIMEOUT, PostFailure, PostRequest, Transport};
use super::event::PostHogEventPayload;
use super::logger::PostHogLogger;
use super::sink::PostHogSink;
use crate::finops::pointfive::upload::url_check::UploadUrlCheck;

pub const OK_STATUS: u16 = 200;
pub const FAILURE_STATUS: u16 = 400;

// should_use_posthog_mock patches every PostHog host, and a custom host keeps its live client
const MOCK_URL_MARKER: &str = "posthog.com";

#[derive(Clone)]
pub struct QueuedEvent {
    pub event: PostHogEventPayload,
    pub api_key: SecretString,
    pub api_url: String,
    pub request_url: bool,
}

pub struct CredentialGroup {
    pub api_key: SecretString,
    pub api_url: String,
    pub request_url: bool,
    pub events: Vec<PostHogEventPayload>,
}

#[derive(Debug, Error)]
pub enum PostHogSendError {
    #[error("the PostHog batch payload could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Transport(#[from] PostFailure),
    #[error("the request-supplied PostHog URL was refused")]
    RefusedUrl(#[source] UserUrlError),
    #[error("the validated PostHog Host header is not a valid header value")]
    HostHeader(#[source] InvalidHeaderValue),
}

#[derive(Serialize)]
struct PostHogBatchPayload<'batch> {
    api_key: &'batch str,
    batch: &'batch [PostHogEventPayload],
}

fn same_key(left: &SecretString, right: &SecretString) -> bool {
    bool::from(
        left.expose_secret()
            .as_bytes()
            .ct_eq(right.expose_secret().as_bytes()),
    )
}

#[must_use]
pub fn group_events(events: Vec<QueuedEvent>) -> Vec<CredentialGroup> {
    let mut groups: Vec<CredentialGroup> = Vec::new();
    for item in events {
        let shared = groups
            .iter_mut()
            .find(|group| group.api_url == item.api_url && same_key(&group.api_key, &item.api_key));
        match shared {
            Some(group) => group.events.push(item.event),
            None => groups.push(CredentialGroup {
                api_key: item.api_key,
                api_url: item.api_url,
                request_url: item.request_url,
                events: vec![item.event],
            }),
        }
    }
    groups
}

pub async fn send_group(
    transport: &dyn Transport,
    mock_mode: bool,
    url_check: &dyn UploadUrlCheck,
    group: &CredentialGroup,
) -> Result<u16, PostHogSendError> {
    let mut url = format!("{}/batch/", group.api_url.trim_end_matches('/'));
    if mock_mode && url.to_ascii_lowercase().contains(MOCK_URL_MARKER) {
        return Ok(OK_STATUS);
    }
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if group.request_url {
        let validated = url_check
            .check(&url)
            .await
            .map_err(PostHogSendError::RefusedUrl)?;
        url = validated.url;
        if let Some(host) = validated.host_header {
            let value = HeaderValue::try_from(host).map_err(PostHogSendError::HostHeader)?;
            headers.insert(HOST, value);
        }
    }
    let encoded = serde_json::to_vec(&PostHogBatchPayload {
        api_key: group.api_key.expose_secret(),
        batch: &group.events,
    })
    .map_err(PostHogSendError::Encode)?;
    let status = transport
        .post(PostRequest {
            url,
            headers,
            body: encoded,
            timeout: LOGGING_TIMEOUT,
        })
        .await?;
    Ok(status)
}

pub async fn log_async_event(logger: &PostHogLogger, event: &LoggerEvent) {
    let queued = match logger.queued_event(event) {
        Ok(queued) => queued,
        Err(error) => {
            tracing::error!(%error, "PostHog Layer Error");
            return;
        }
    };
    let enqueued = logger.sink.queue.enqueue(queued);
    tracing::debug!(
        flush_interval = ?logger.settings.batch.flush_interval,
        "PostHog, event added to queue. Will flush on the interval or at the batch size"
    );
    if enqueued.depth >= logger.settings.batch.batch_size {
        logger.flush_queue().await;
    }
}

// LiteLLM's async_send_batch aborts the remaining credential groups on the first failed post
pub async fn send_batches(sink: &PostHogSink, events: Vec<QueuedEvent>) {
    for group in group_events(events) {
        match send_group(
            sink.transport.as_ref(),
            sink.mock_mode,
            sink.url_check.as_ref(),
            &group,
        )
        .await
        {
            Ok(OK_STATUS) => {}
            Err(PostHogSendError::RefusedUrl(refused)) => {
                tracing::warn!(error = %refused, "PostHog: refused a request-supplied PostHog URL");
            }
            Ok(status) => {
                tracing::error!(status, "PostHog Error sending batch API");
                return;
            }
            Err(error) => {
                tracing::error!(%error, "PostHog Error sending batch API");
                return;
            }
        }
    }
}

pub async fn periodic_flush(
    sink: PostHogSink,
    interval: Duration,
    stopped: CancellationToken,
    tasks: TaskTracker,
) {
    loop {
        let shutdown = timeout(interval, stopped.cancelled()).await;
        if shutdown.is_ok() || tasks.is_closed() {
            return;
        }
        tracing::debug!(?interval, "CustomLogger periodic flush");
        sink.flush().await;
    }
}

pub struct PostHogBatch<'sink> {
    pub sink: &'sink PostHogSink,
}

impl BatchDelivery<QueuedEvent> for PostHogBatch<'_> {
    async fn deliver(
        &self,
        batch: &[QueuedEvent],
        settled: &AtomicUsize,
    ) -> Result<Vec<QueuedEvent>, BatchCancelled<QueuedEvent>> {
        // PostHog events are never retried: LiteLLM clears its queue once a send attempt returns
        settled.fetch_add(batch.len(), Ordering::SeqCst);
        send_batches(self.sink, batch.to_vec()).await;
        Ok(Vec::new())
    }
}
