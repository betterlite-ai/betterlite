use std::sync::Arc;

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use betterlite_logging::trace_types::{SpanDetail, SpanErrorPage, Trace, TracePage, TraceScope};
use betterlite_server::tracing_routes::access::Tenant;
use betterlite_server::tracing_routes::query::{QueryRunError, QueryScope, TraceQueryStore};
use betterlite_server::tracing_routes::receiver::{IngestError, TraceReadError, TraceRuntime};
use betterlite_tracing::storage::trace_store::SpanErrorTarget;
use secrecy::{ExposeSecret as _, SecretString};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::Script;

const DEFAULT_MESSAGE: &str = "the trace read failed";

pub(super) struct ScriptedRuntime {
    pub(super) script: Script,
}

#[async_trait]
impl TraceRuntime for ScriptedRuntime {
    async fn ingest(
        &self,
        body: Body,
        _content_type: Option<&str>,
        _content_encoding: Option<&str>,
        _tenant: &Tenant,
    ) -> Result<usize, IngestError> {
        let _payload = to_bytes(body, usize::MAX)
            .await
            .map_err(IngestError::BodyRead)?;
        let kind = self.script.decoder.get("kind").and_then(Value::as_str);
        let message = self
            .script
            .decoder
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("OTLP payload rejected")
            .to_owned();
        match kind {
            Some("too_large" | "overflow") => Err(IngestError::TooLarge(message)),
            Some("malformed") => Err(IngestError::Invalid(message)),
            Some("overloaded") => Err(IngestError::Overloaded),
            _other => Ok(1),
        }
    }

    async fn list_traces(
        &self,
        _scope: &TraceScope,
        _window: (i64, i64),
        _cursor: Option<&str>,
    ) -> Result<TracePage, TraceReadError> {
        read_answer(&self.script.list_traces)
    }

    async fn get_trace(
        &self,
        _trace_id: &str,
        _scope: &TraceScope,
        _trace_ref: &str,
    ) -> Result<Option<Trace>, TraceReadError> {
        read_answer(&self.script.get_trace)
    }

    async fn get_span(
        &self,
        _trace_id: &str,
        _span_id: &str,
        _scope: &TraceScope,
        _trace_ref: &str,
    ) -> Result<Option<SpanDetail>, TraceReadError> {
        let script = &self.script;
        let section = &script.get_span;
        read_answer(section)
    }

    async fn get_span_error(
        &self,
        _target: SpanErrorTarget<'_>,
        _scope: &TraceScope,
        _cursor: Option<&str>,
    ) -> Result<Option<SpanErrorPage>, TraceReadError> {
        read_answer(&self.script.get_span_error)
    }

    fn query_store(&self) -> Arc<dyn TraceQueryStore> {
        Arc::new(CannedQueries)
    }
}

fn read_answer<Payload>(section: &Value) -> Result<Payload, TraceReadError>
where
    Payload: DeserializeOwned,
{
    if let Some(value) = section.get("value") {
        return serde_json::from_value(value.clone()).map_err(|_error| TraceReadError::Storage);
    }
    let Some(error) = section.get("error") else {
        return Err(TraceReadError::Storage);
    };
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_MESSAGE);
    if error.get("kind").and_then(Value::as_str) == Some("invalid_cursor") {
        return Err(TraceReadError::InvalidCursor(message.to_owned()));
    }
    Err(TraceReadError::Storage)
}

struct CannedQueries;

#[async_trait]
impl TraceQueryStore for CannedQueries {
    async fn query_sql(
        &self,
        _sql: &str,
        scope: &QueryScope,
        secret: &SecretString,
    ) -> Result<String, QueryRunError> {
        Ok(echo(scope, secret))
    }

    async fn query_help(
        &self,
        scope: &QueryScope,
        secret: &SecretString,
    ) -> Result<String, QueryRunError> {
        Ok(echo(scope, secret))
    }
}

fn echo(scope: &QueryScope, secret: &SecretString) -> String {
    serde_json::to_string(&json!({"scope": scope, "secret": secret.expose_secret()}))
        .unwrap_or_else(|_error| "{}".to_owned())
}
