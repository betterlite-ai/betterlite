use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use betterlite_logging::trace_types::{SpanDetail, SpanErrorPage, Trace, TracePage, TraceScope};
use betterlite_server::tracing_routes::access::{Tenant, TraceClock};
use betterlite_server::tracing_routes::query::{QueryRunError, QueryScope, TraceQueryStore};
use betterlite_server::tracing_routes::receiver::{IngestError, TraceReadError, TraceRuntime};
use betterlite_tracing::storage::trace_store::SpanErrorTarget;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::inputs::{DecoderKind, ErrorKind, ErrorScript, QueryScript, ReadScript, Script};

const DEFAULT_DECODER_REJECTION: &str = "OTLP payload rejected";
const DEFAULT_READ_FAILURE: &str = "the trace read failed";
const DEFAULT_QUERY_FAILURE: &str = "the trace query failed";

pub(super) struct FixedClock {
    pub(super) now_ms: i64,
}

impl TraceClock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.now_ms
    }
}

pub(super) struct ScriptedRuntime {
    pub(super) script: Script,
    pub(super) seen: Arc<Mutex<Option<Value>>>,
    pub(super) queries: Arc<ScriptedQueries>,
}

#[async_trait]
impl TraceRuntime for ScriptedRuntime {
    async fn ingest(
        &self,
        body: Body,
        _content_type: Option<&str>,
        _content_encoding: Option<&str>,
        tenant: &Tenant,
    ) -> Result<usize, IngestError> {
        let payload = to_bytes(body, usize::MAX)
            .await
            .map_err(IngestError::BodyRead)?;
        let kind = self.script.decoder.kind;
        let message = self.decoder_message();
        match kind {
            DecoderKind::TooLarge | DecoderKind::Overflow => Err(IngestError::TooLarge(message)),
            DecoderKind::Malformed => Err(IngestError::Invalid(message)),
            DecoderKind::Overloaded => Err(IngestError::Overloaded),
            DecoderKind::Ok => {
                let _consumed = payload.len();
                let mut rows = self.script.decoder.rows.clone();
                tenant.stamp_rows(&mut rows);
                self.record(json!({
                    "kind": "ingest",
                    "tenant": tenant_json(tenant),
                    "rows": rows,
                }));
                Ok(rows.len())
            }
        }
    }

    async fn list_traces(
        &self,
        scope: &TraceScope,
        window: (i64, i64),
        cursor: Option<&str>,
    ) -> Result<TracePage, TraceReadError> {
        self.record(json!({
            "kind": "list_traces",
            "scope": scope,
            "start_ms": window.0,
            "end_ms": window.1,
            "cursor": cursor,
        }));
        read_answer(&self.script.list_traces)
    }

    async fn get_trace(
        &self,
        trace_id: &str,
        scope: &TraceScope,
        trace_ref: &str,
    ) -> Result<Option<Trace>, TraceReadError> {
        self.record(json!({
            "kind": "get_trace",
            "trace_id": trace_id,
            "trace_ref": trace_ref,
            "scope": scope,
        }));
        read_answer(&self.script.get_trace)
    }

    async fn get_span(
        &self,
        trace_id: &str,
        span_id: &str,
        scope: &TraceScope,
        trace_ref: &str,
    ) -> Result<Option<SpanDetail>, TraceReadError> {
        self.record(json!({
            "kind": "get_span",
            "trace_id": trace_id,
            "span_id": span_id,
            "trace_ref": trace_ref,
            "scope": scope,
        }));
        read_answer(&self.script.get_span)
    }

    async fn get_span_error(
        &self,
        target: SpanErrorTarget<'_>,
        scope: &TraceScope,
        cursor: Option<&str>,
    ) -> Result<Option<SpanErrorPage>, TraceReadError> {
        self.record(json!({
            "kind": "get_span_error",
            "trace_id": target.trace_id,
            "span_id": target.span_id,
            "trace_ref": target.trace_ref,
            "cursor": cursor,
            "scope": scope,
        }));
        read_answer(&self.script.get_span_error)
    }

    fn query_store(&self) -> Arc<dyn TraceQueryStore> {
        let store: Arc<dyn TraceQueryStore> = Arc::clone(&self.queries);
        store
    }
}

impl ScriptedRuntime {
    fn decoder_message(&self) -> String {
        self.script
            .decoder
            .message
            .clone()
            .unwrap_or_else(|| DEFAULT_DECODER_REJECTION.to_owned())
    }

    fn record(&self, value: Value) {
        if let Ok(mut seen) = self.seen.lock() {
            *seen = Some(value);
        }
    }
}

pub(super) struct ScriptedQueries {
    pub(super) sql: QueryScript,
    pub(super) help: QueryScript,
    pub(super) seen: Arc<Mutex<Option<Value>>>,
}

#[async_trait]
impl TraceQueryStore for ScriptedQueries {
    async fn query_sql(
        &self,
        sql: &str,
        scope: &QueryScope,
        secret: &SecretString,
    ) -> Result<String, QueryRunError> {
        self.record(json!({
            "kind": "query",
            "sql": sql,
            "scope": scope,
            "secret": secret.expose_secret(),
        }));
        query_answer(&self.sql)
    }

    async fn query_help(
        &self,
        scope: &QueryScope,
        secret: &SecretString,
    ) -> Result<String, QueryRunError> {
        self.record(json!({
            "kind": "query_help",
            "scope": scope,
            "secret": secret.expose_secret(),
        }));
        query_answer(&self.help)
    }
}

impl ScriptedQueries {
    fn record(&self, value: Value) {
        if let Ok(mut seen) = self.seen.lock() {
            *seen = Some(value);
        }
    }
}

fn read_answer<Payload>(script: &ReadScript<Payload>) -> Result<Payload, TraceReadError>
where
    Payload: Clone,
{
    match (&script.value, &script.error) {
        (Some(value), _) => Ok(value.clone()),
        (None, Some(error)) => Err(read_error(error)),
        (None, None) => Err(TraceReadError::Storage),
    }
}

fn read_error(error: &ErrorScript) -> TraceReadError {
    let message = error
        .message
        .clone()
        .unwrap_or_else(|| DEFAULT_READ_FAILURE.to_owned());
    match error.kind {
        ErrorKind::InvalidCursor => TraceReadError::InvalidCursor(message),
        ErrorKind::Storage | ErrorKind::Invalid | ErrorKind::Unavailable => TraceReadError::Storage,
    }
}

fn query_answer(script: &QueryScript) -> Result<String, QueryRunError> {
    match (&script.value, &script.error) {
        (Some(value), _) => Ok(value.clone()),
        (None, Some(error)) => {
            let message = error
                .message
                .clone()
                .unwrap_or_else(|| DEFAULT_QUERY_FAILURE.to_owned());
            match error.kind {
                ErrorKind::Unavailable => Err(QueryRunError::Unavailable(message)),
                ErrorKind::Invalid | ErrorKind::InvalidCursor | ErrorKind::Storage => {
                    Err(QueryRunError::Invalid(message))
                }
            }
        }
        (None, None) => Err(QueryRunError::Invalid(DEFAULT_QUERY_FAILURE.to_owned())),
    }
}

fn tenant_json(tenant: &Tenant) -> Value {
    json!({
        "team_id": tenant.team_id,
        "api_key_hash": tenant.api_key_hash,
        "org_id": tenant.org_id,
    })
}
