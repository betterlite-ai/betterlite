use std::error::Error as StdError;
use std::str;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::EnvironmentLookup;
use http::HeaderMap;
use http::header::{CONTENT_TYPE, HeaderValue};
use serde::Serialize;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_util::task::TaskTracker;
use url::{ParseError, Url};

use super::{ALERT_HEADER_VALUE, WebhookBatch};
use crate::alert_digest::{DigestBuckets, digest_queue_items};
use crate::alert_settings::records::AlertQueueItem;
use crate::batch::logger::{BatchQueue, FlushReport};
use crate::http_analytics_loggers::delivery::{DELIVERY_DEADLINE, Delivery, PostRequest};

mod tests;

pub const WEBHOOK_URL_ENV: &str = "WEBHOOK_URL";
pub const PROXY_BASE_URL_ENV: &str = "PROXY_BASE_URL";

#[derive(Debug, thiserror::Error)]
pub enum WebhookAlertError<E>
where
    E: StdError + 'static,
{
    #[error("WEBHOOK_URL is not set")]
    MissingUrl,
    #[error("WEBHOOK_URL is not a valid URL")]
    Url(#[source] ParseError),
    #[error("the webhook alert could not be written as JSON")]
    Encode(#[source] serde_json::Error),
    #[error("the webhook alert could not be delivered")]
    Transport(#[source] E),
    #[error("the webhook answered with status {status}")]
    Status { status: u16 },
}

#[derive(Default)]
pub struct FlushTask {
    task: Mutex<Option<JoinHandle<()>>>,
}

pub struct AlertDelivery<D> {
    pub queue: BatchQueue<AlertQueueItem>,
    pub digests: DigestBuckets,
    pub delivery: D,
    pub log_to_console: bool,
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    pub flush_task: FlushTask,
}

impl<D> AlertDelivery<D>
where
    D: Delivery + 'static,
{
    pub fn ensure_started(self: &Arc<Self>, tasks: &TaskTracker) {
        let mut task = self
            .flush_task
            .task
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if task.as_ref().is_some_and(|handle| !handle.is_finished()) {
            return;
        }
        let runner = Arc::clone(self);
        let owner = tasks.clone();
        *task = Some(tasks.spawn(async move { runner.periodic_flush(owner).await }));
    }

    async fn periodic_flush(self: Arc<Self>, tasks: TaskTracker) {
        while !tasks.is_closed() {
            sleep(self.queue.settings.flush_interval).await;
            self.flush_digest_buckets();
            let _report = self.async_send_batch().await;
        }
    }

    pub fn flush_digest_buckets(&self) {
        let proxy_base_url = self.environment.lookup(PROXY_BASE_URL_ENV);
        let expired = self.digests.flush_expired();
        for item in digest_queue_items(&expired, proxy_base_url.as_deref()) {
            let _enqueued = self.queue.enqueue(item);
        }
    }

    #[must_use]
    pub async fn async_send_batch(&self) -> FlushReport {
        let batch = WebhookBatch {
            delivery: &self.delivery,
            log_to_console: self.log_to_console,
        };
        self.queue.flush(&batch).await
    }

    pub async fn send_webhook_alert<E>(
        &self,
        event: &E,
    ) -> Result<bool, WebhookAlertError<D::Error>>
    where
        E: Serialize + Sync + ?Sized,
    {
        let Some(url) = self
            .environment
            .lookup(WEBHOOK_URL_ENV)
            .filter(|value| !value.is_empty())
        else {
            return Err(WebhookAlertError::MissingUrl);
        };
        let parsed_url = Url::parse(&url).map_err(WebhookAlertError::Url)?;
        let body = serde_json::to_vec(event).map_err(WebhookAlertError::Encode)?;
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(ALERT_HEADER_VALUE));
        let response = self
            .delivery
            .post(PostRequest {
                url: parsed_url,
                headers,
                body,
                deadline: DELIVERY_DEADLINE,
            })
            .await
            .map_err(WebhookAlertError::Transport)?;
        if response.status == 200 {
            return Ok(true);
        }
        if response.status >= 400 {
            // AsyncHTTPHandler.post raises raise_for_status, so the source aborts the alert instead of returning False
            return Err(WebhookAlertError::Status {
                status: response.status,
            });
        }
        tracing::error!(
            status = response.status,
            detail = str::from_utf8(&response.body).unwrap_or_default(),
            "the webhook alert was rejected"
        );
        Ok(false)
    }
}
