use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::slice;
use std::str;
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::sleep;
use tokio_util::task::TaskTracker;
use url::Url;

use crate::http_analytics_loggers::delivery::{
    DELIVERY_DEADLINE, Delivery, PostRequest, PostResponse,
};
use crate::opik::payload::{build_opik_payload, classify_payload};

use super::callback::{Call, should_skip_event};
use super::error::OpikError;
use super::native::{NativeClientSource, NativeOpikClient};
use super::queue::PayloadQueue;
use super::record::{Payload, span_record, span_wire, trace_record, trace_wire};
use super::settings::{LoggerOptions, OpikSettings, OpikSettingsError, resolve_settings};

pub struct LoggerDependencies<'context, D> {
    pub delivery: D,
    pub native: &'context dyn NativeClientSource,
    pub tasks: Option<&'context TaskTracker>,
}

pub struct OpikLogger<D> {
    pub(crate) settings: OpikSettings,
    pub(crate) delivery: D,
    native: Option<Arc<dyn NativeOpikClient>>,
    queue: PayloadQueue,
    flush_lock: Option<Arc<AsyncMutex<()>>>,
}

#[derive(Serialize)]
struct TraceBatch<'payload> {
    traces: &'payload [Payload],
}

#[derive(Serialize)]
struct SpanBatch<'payload> {
    spans: &'payload [Payload],
}

impl<D> OpikLogger<D>
where
    D: Delivery + 'static,
{
    pub fn build(
        options: &LoggerOptions<'_>,
        environment: &BTreeMap<String, String>,
        config_file: Option<&str>,
        dependencies: LoggerDependencies<'_, D>,
    ) -> Result<Arc<Self>, OpikSettingsError> {
        let settings = resolve_settings(options, environment, config_file)?;
        let max_queue_size = settings.batch.max_queue_size;
        let native = dependencies.native.get_client_cached();
        let flush_lock = dependencies
            .tasks
            .map(|_tasks| Arc::new(AsyncMutex::new(())));
        let logger = Arc::new(Self {
            queue: PayloadQueue::new(max_queue_size),
            settings,
            delivery: dependencies.delivery,
            native,
            flush_lock,
        });
        let Some(tasks) = dependencies.tasks else {
            tracing::warn!(
                "OpikLogger - Asynchronous processing not initialized as we are not running in an async context"
            );
            return Ok(logger);
        };
        let periodic = Arc::clone(&logger);
        let owner = tasks.clone();
        let _task = tasks.spawn(async move {
            periodic.periodic_flush(&owner).await;
        });
        Ok(logger)
    }

    pub async fn async_log_success_event(&self, call: Call<'_>) {
        if should_skip_event(call.kwargs) {
            return;
        }
        if let Err(error) = self.async_deliver(call).await {
            tracing::error!(%error, "OpikLogger failed to log success event");
        }
    }

    pub async fn log_success_event(&self, call: Call<'_>) {
        if should_skip_event(call.kwargs) {
            return;
        }
        if let Err(error) = self.sync_deliver(call).await {
            tracing::error!(%error, "OpikLogger failed to log success event");
        }
    }

    async fn async_deliver(&self, call: Call<'_>) -> Result<(), OpikError<D::Error>> {
        let (trace, span) = build_opik_payload(
            call.kwargs,
            call.response,
            call.start_time,
            call.end_time,
            &self.settings.project_name,
        );
        if let Some(native) = &self.native {
            return native_deliver(native.as_ref(), trace.as_ref(), &span);
        }
        if let Some(trace_payload) = trace {
            self.queue.enqueue(trace_payload);
        }
        let depth = self.queue.enqueue(span);
        tracing::debug!(
            flush_interval = ?self.settings.batch.flush_interval,
            "OpikLogger added event to log_queue - it will flush on the flush interval"
        );
        if depth >= self.settings.batch.batch_size {
            self.flush_queue().await;
        }
        Ok(())
    }

    async fn sync_deliver(&self, call: Call<'_>) -> Result<(), OpikError<D::Error>> {
        let (trace, span) = build_opik_payload(
            call.kwargs,
            call.response,
            call.start_time,
            call.end_time,
            &self.settings.project_name,
        );
        if let Some(native) = &self.native {
            return native_deliver(native.as_ref(), trace.as_ref(), &span);
        }
        if let Some(trace_payload) = trace {
            let wire = trace_wire(&trace_payload);
            let batch = TraceBatch {
                traces: slice::from_ref(&wire),
            };
            self.sync_send(&self.settings.trace_url, &batch).await;
        }
        let wire = span_wire(&span);
        let batch = SpanBatch {
            spans: slice::from_ref(&wire),
        };
        self.sync_send(&self.settings.span_url, &batch).await;
        Ok(())
    }

    async fn sync_send<B>(&self, url: &Url, batch: &B)
    where
        B: Serialize + Sync,
    {
        if let Err(error) = self.send_expecting_204(url, batch).await {
            tracing::error!(%error, "OpikLogger failed to send batch");
        }
    }

    pub(super) async fn send_expecting_204<B>(
        &self,
        url: &Url,
        batch: &B,
    ) -> Result<(), OpikError<D::Error>>
    where
        B: Serialize + Sync,
    {
        let response = self.post(url, batch).await?;
        if response.status == 204 {
            return Ok(());
        }
        Err(OpikError::UnexpectedStatus {
            status: response.status,
            detail: str::from_utf8(&response.body)
                .unwrap_or_default()
                .to_owned(),
        })
    }

    async fn submit_batch<B>(&self, url: &Url, batch: &B, events: usize)
    where
        B: Serialize + Sync,
    {
        match self.post(url, batch).await {
            Ok(response) if response.status >= 300 => {
                tracing::error!(
                    status = response.status,
                    detail = str::from_utf8(&response.body).unwrap_or_default(),
                    "OpikLogger - Error: the Opik API rejected the batch"
                );
            }
            Ok(_response) => {
                tracing::info!(events, "OpikLogger - Opik events submitted");
            }
            Err(error) => {
                tracing::error!(%error, "OpikLogger failed to send batch");
            }
        }
    }

    async fn post<B>(&self, url: &Url, batch: &B) -> Result<PostResponse, OpikError<D::Error>>
    where
        B: Serialize + Sync,
    {
        let body = serde_json::to_vec(batch).map_err(OpikError::Encode)?;
        let request = PostRequest {
            url: url.clone(),
            headers: self.settings.headers.clone(),
            body,
            deadline: DELIVERY_DEADLINE,
        };
        self.delivery
            .post(request)
            .await
            .map_err(OpikError::Transport)
    }

    pub async fn async_send_batch(&self) {
        tracing::info!("Calling async_send_batch");
        let entries = self.queue.snapshot();
        if entries.is_empty() {
            return;
        }
        let events = entries.len();
        let (traces, spans) = classify_payload(&entries);
        if !traces.is_empty() {
            let batch = TraceBatch { traces: &traces };
            self.submit_batch(&self.settings.trace_url, &batch, events)
                .await;
            tracing::info!(count = traces.len(), "Sent traces");
        }
        if !spans.is_empty() {
            let batch = SpanBatch { spans: &spans };
            self.submit_batch(&self.settings.span_url, &batch, events)
                .await;
            tracing::info!(count = spans.len(), "Sent spans");
        }
    }

    pub async fn flush_queue(&self) {
        let Some(lock) = &self.flush_lock else {
            return;
        };
        let _guard = lock.lock().await;
        let entries = self.queue.snapshot();
        if entries.is_empty() {
            return;
        }
        tracing::debug!(
            events = entries.len(),
            "CustomLogger: Flushing batch of events"
        );
        self.async_send_batch().await;
        self.queue.clear();
    }

    async fn periodic_flush(self: Arc<Self>, tasks: &TaskTracker) {
        while !tasks.is_closed() {
            sleep(self.settings.batch.flush_interval).await;
            tracing::debug!(
                flush_interval = ?self.settings.batch.flush_interval,
                "CustomLogger periodic flush"
            );
            self.flush_queue().await;
        }
    }
}

fn native_deliver<E>(
    native: &dyn NativeOpikClient,
    trace_payload: Option<&Payload>,
    span: &Payload,
) -> Result<(), OpikError<E>>
where
    E: StdError + 'static,
{
    if let Some(trace) = trace_payload {
        native.trace(&trace_record(trace)?)?;
    }
    native.span(&span_record(span)?)?;
    Ok(())
}
