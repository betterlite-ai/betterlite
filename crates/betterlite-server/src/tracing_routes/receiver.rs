use std::error::Error;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::{Body, Bytes};
use betterlite_logging::otlp_errors::OtlpError;
use betterlite_logging::otlp_normalize::{OTLP_MAX_BODY_BYTES, SpanCodec, SpanRow, decode_otlp};
use betterlite_logging::trace_types::{SpanDetail, SpanErrorPage, Trace, TracePage, TraceScope};
use betterlite_tracing::storage::error::StorageError;
use betterlite_tracing::storage::trace_store::{SpanErrorTarget, TraceStorage, TraceStore};
use futures_util::StreamExt as _;
use thiserror::Error;
use tokio::sync::Semaphore;
use tokio::time::timeout;

use crate::tracing_routes::access::Tenant;
use crate::tracing_routes::query::TraceQueryStore;

pub const OTLP_MAX_CONCURRENT_INGESTS: usize = 2;
pub const OTLP_BODY_READ_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Error)]
pub enum IngestError {
    #[error("OTLP ingestion is at capacity")]
    Overloaded,
    #[error("{0}")]
    TooLarge(String),
    #[error("{0}")]
    Invalid(String),
    #[error("the request body could not be read")]
    BodyRead(#[source] axum::Error),
    #[error("trace storage is unavailable")]
    Storage,
}

#[derive(Debug, Error)]
pub enum TraceReadError {
    #[error("{0}")]
    InvalidCursor(String),
    #[error("trace storage is unavailable")]
    Storage,
}

#[derive(Debug, Error)]
pub enum ReceiverConfigError {
    #[error("OTLP ingestion concurrency must be positive")]
    Concurrency,
}

pub trait TraceSpanSink: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn insert_spans(
        &self,
        rows: &[SpanRow],
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
    fn overflow_message(error: &Self::Error) -> Option<String>;
}

#[async_trait]
pub trait TraceRuntime: Send + Sync {
    async fn ingest(
        &self,
        body: Body,
        content_type: Option<&str>,
        content_encoding: Option<&str>,
        tenant: &Tenant,
    ) -> Result<usize, IngestError>;
    async fn list_traces(
        &self,
        scope: &TraceScope,
        window: (i64, i64),
        cursor: Option<&str>,
    ) -> Result<TracePage, TraceReadError>;
    async fn get_trace(
        &self,
        trace_id: &str,
        scope: &TraceScope,
        trace_ref: &str,
    ) -> Result<Option<Trace>, TraceReadError>;
    async fn get_span(
        &self,
        trace_id: &str,
        span_id: &str,
        scope: &TraceScope,
        trace_ref: &str,
    ) -> Result<Option<SpanDetail>, TraceReadError>;
    async fn get_span_error(
        &self,
        target: SpanErrorTarget<'_>,
        scope: &TraceScope,
        cursor: Option<&str>,
    ) -> Result<Option<SpanErrorPage>, TraceReadError>;
    fn query_store(&self) -> Arc<dyn TraceQueryStore>;
}

pub struct StoreTraceReceiver<S, C, Q> {
    store: TraceStore<S>,
    decoder: C,
    queries: Arc<Q>,
    slots: Semaphore,
    body_read_timeout: Duration,
}

impl<S, C, Q> StoreTraceReceiver<S, C, Q>
where
    S: TraceSpanSink,
    C: Send + Sync,
    Q: Send + Sync,
{
    pub fn new(
        store: TraceStore<S>,
        decoder: C,
        queries: Arc<Q>,
        max_concurrent_ingests: usize,
        body_read_timeout: Duration,
    ) -> Result<Self, ReceiverConfigError> {
        if max_concurrent_ingests == 0 {
            return Err(ReceiverConfigError::Concurrency);
        }
        Ok(Self {
            store,
            decoder,
            queries,
            slots: Semaphore::new(max_concurrent_ingests),
            body_read_timeout,
        })
    }

    async fn read_payload(&self, body: Body) -> Result<Bytes, IngestError> {
        match timeout(self.body_read_timeout, read_bounded(body)).await {
            Ok(result) => result,
            Err(_elapsed) => Err(IngestError::Overloaded),
        }
    }

    async fn insert(&self, rows: &[SpanRow]) -> Result<(), IngestError> {
        match self.store.storage.insert_spans(rows).await {
            Ok(()) => Ok(()),
            Err(error) => S::overflow_message(&error).map_or_else(
                || Err(IngestError::Storage),
                |message| Err(IngestError::TooLarge(message)),
            ),
        }
    }
}

#[async_trait]
impl<S, C, Q> TraceRuntime for StoreTraceReceiver<S, C, Q>
where
    S: TraceStorage + TraceSpanSink + Send + Sync,
    C: SpanCodec + Send + Sync,
    Q: TraceQueryStore + 'static,
{
    async fn ingest(
        &self,
        body: Body,
        content_type: Option<&str>,
        content_encoding: Option<&str>,
        tenant: &Tenant,
    ) -> Result<usize, IngestError> {
        let _slot = self
            .slots
            .try_acquire()
            .map_err(|_error| IngestError::Overloaded)?;
        let payload = self.read_payload(body).await?;
        let mut rows = match decode_otlp(&self.decoder, &payload, content_type, content_encoding) {
            Ok(rows) => rows,
            Err(error) => return Err(decode_failure(&error)),
        };
        tenant.stamp_rows(&mut rows);
        self.insert(&rows).await?;
        Ok(rows.len())
    }

    async fn list_traces(
        &self,
        scope: &TraceScope,
        window: (i64, i64),
        cursor: Option<&str>,
    ) -> Result<TracePage, TraceReadError> {
        self.store
            .list_traces(scope, window, cursor)
            .await
            .map_err(read_failure)
    }

    async fn get_trace(
        &self,
        trace_id: &str,
        scope: &TraceScope,
        trace_ref: &str,
    ) -> Result<Option<Trace>, TraceReadError> {
        self.store
            .get_trace(trace_id, scope, trace_ref)
            .await
            .map_err(read_failure)
    }

    async fn get_span(
        &self,
        trace_id: &str,
        span_id: &str,
        scope: &TraceScope,
        trace_ref: &str,
    ) -> Result<Option<SpanDetail>, TraceReadError> {
        let store = &self.store;
        let found = store.get_span(trace_id, span_id, scope, trace_ref).await;
        found.map_err(read_failure)
    }

    async fn get_span_error(
        &self,
        target: SpanErrorTarget<'_>,
        scope: &TraceScope,
        cursor: Option<&str>,
    ) -> Result<Option<SpanErrorPage>, TraceReadError> {
        self.store
            .get_span_error(target, scope, cursor)
            .await
            .map_err(read_failure)
    }

    fn query_store(&self) -> Arc<dyn TraceQueryStore> {
        Arc::clone(&self.queries)
    }
}

async fn read_bounded(body: Body) -> Result<Bytes, IngestError> {
    let mut stream = body.into_data_stream();
    let mut collected: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(IngestError::BodyRead)?;
        if collected.len().saturating_add(bytes.len()) > OTLP_MAX_BODY_BYTES {
            return Err(IngestError::TooLarge(format!(
                "OTLP body exceeds {OTLP_MAX_BODY_BYTES} bytes"
            )));
        }
        collected.extend_from_slice(&bytes);
    }
    Ok(Bytes::from(collected))
}

fn decode_failure(error: &OtlpError) -> IngestError {
    match error {
        OtlpError::BodyTooLarge | OtlpError::SpansTooLarge(_) => {
            IngestError::TooLarge(error.to_string())
        }
        OtlpError::UnsupportedEncoding | OtlpError::InvalidGzip(_) | OtlpError::InvalidSpans(_) => {
            IngestError::Invalid(error.to_string())
        }
    }
}

fn read_failure<Source>(error: StorageError<Source>) -> TraceReadError
where
    Source: Error,
{
    match error {
        StorageError::Cursor(source) => TraceReadError::InvalidCursor(source.to_string()),
        StorageError::Query(source) => {
            tracing::warn!(error = %source, "trace storage read failed");
            TraceReadError::Storage
        }
    }
}
