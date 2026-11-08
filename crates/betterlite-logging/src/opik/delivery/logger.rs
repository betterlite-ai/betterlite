use std::collections::BTreeMap;
use std::slice;
use std::str;
use std::sync::Arc;

use serde::Serialize;
use tokio::time::sleep;
use tokio_util::task::TaskTracker;
use url::Url;

use crate::batch::logger::BatchQueue;
use crate::http_analytics_loggers::delivery::{
    DELIVERY_DEADLINE, Delivery, PostRequest, PostResponse,
};
use crate::opik::payload::{build_opik_payload, classify_payload};

use super::callback::{Call, should_skip_event};
use super::error::{NativeDeliveryError, OpikError};
use super::native::{NativeClientSource, NativeOpikClient};
use super::queue::OpikBatch;
use super::record::{
    Payload, SPAN_OPTIONAL_FIELDS, SpanRecord, TRACE_OPTIONAL_FIELDS, TraceRecord, wire_payload,
};
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
    queue: BatchQueue<Payload>,
    flush_enabled: bool,
}

#[derive(Clone, Copy)]
enum DeliveryMode {
    Sync,
    Async,
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
        let native = dependencies.native.get_client_cached();
        let flush_enabled = dependencies.tasks.is_some();
        let logger = Arc::new(Self {
            queue: BatchQueue::new(settings.batch),
            settings,
            delivery: dependencies.delivery,
            native,
            flush_enabled,
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
        self.deliver(call, DeliveryMode::Async).await;
    }

    pub async fn log_success_event(&self, call: Call<'_>) {
        if should_skip_event(call.kwargs) {
            return;
        }
        self.deliver(call, DeliveryMode::Sync).await;
    }

    async fn deliver(&self, call: Call<'_>, mode: DeliveryMode) {
        let (trace, span) = build_opik_payload(
            call.kwargs,
            call.response,
            call.start_time,
            call.end_time,
            &self.settings.project_name,
        );
        if let Some(native) = &self.native {
            if let Err(error) = native_deliver(native.as_ref(), trace.as_ref(), &span) {
                tracing::error!(%error, "OpikLogger failed to log success event");
            }
            return;
        }
        match mode {
            DeliveryMode::Async => self.queue_delivery(trace, span).await,
            DeliveryMode::Sync => self.http_delivery(trace.as_ref(), &span).await,
        }
    }

    async fn queue_delivery(&self, trace: Option<Payload>, span: Payload) {
        if let Some(trace_payload) = trace {
            let _trace_enqueued = self.queue.enqueue(trace_payload);
        }
        let enqueued = self.queue.enqueue(span);
        tracing::debug!(
            flush_interval = ?self.settings.batch.flush_interval,
            "OpikLogger added event to log_queue - it will flush on the flush interval"
        );
        if enqueued.depth >= self.settings.batch.batch_size {
            self.flush_queue().await;
        }
    }

    async fn http_delivery(&self, trace: Option<&Payload>, span: &Payload) {
        if let Some(trace_payload) = trace {
            self.send_trace(trace_payload).await;
        }
        self.send_span(span).await;
    }

    async fn send_trace(&self, trace: &Payload) {
        let wire = wire_payload(trace, &TRACE_OPTIONAL_FIELDS);
        let batch = TraceBatch {
            traces: slice::from_ref(&wire),
        };
        self.sync_send(&self.settings.trace_url, &batch).await;
    }

    async fn send_span(&self, span: &Payload) {
        let wire = wire_payload(span, &SPAN_OPTIONAL_FIELDS);
        let batch = SpanBatch {
            spans: slice::from_ref(&wire),
        };
        self.sync_send(&self.settings.span_url, &batch).await;
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
        let entries = self.queue.pending();
        if entries.is_empty() {
            return;
        }
        self.submit_entries(&entries).await;
    }

    pub(super) async fn submit_entries(&self, entries: &[Payload]) {
        let events = entries.len();
        let (traces, spans) = classify_payload(entries);
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
        if !self.flush_enabled {
            return;
        }
        let pending = self.queue.pending();
        if pending.is_empty() {
            return;
        }
        tracing::debug!(
            events = pending.len(),
            "CustomLogger: Flushing batch of events"
        );
        let batcher = OpikBatch { logger: self };
        self.queue.flush(&batcher).await;
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

fn native_deliver(
    native: &dyn NativeOpikClient,
    trace_payload: Option<&Payload>,
    span: &Payload,
) -> Result<(), NativeDeliveryError> {
    if let Some(trace) = trace_payload {
        native.trace(&TraceRecord::try_from(trace)?)?;
    }
    native.span(&SpanRecord::try_from(span)?)?;
    Ok(())
}
