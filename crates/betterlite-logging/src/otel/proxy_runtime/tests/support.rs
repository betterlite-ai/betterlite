#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use chrono::FixedOffset;
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};

use crate::otel::plumbing::context::context_slots::AnchoredSpan;
use crate::otel::plumbing::exporters::HttpClients;

#[derive(Debug)]
struct NoClient;

#[async_trait]
impl HttpClient for NoClient {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(Box::from("the proxy runtime tests export in memory"))
    }
}

pub(crate) fn http_clients() -> HttpClients {
    let client: Arc<dyn HttpClient> = Arc::new(NoClient);
    HttpClients {
        traces: Arc::clone(&client),
        metrics: Arc::clone(&client),
        logs: client,
    }
}

pub(crate) fn config(source: &str) -> Config {
    ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("the config loads")
}

pub(crate) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn utc_offset() -> FixedOffset {
    FixedOffset::east_opt(0).expect("utc")
}

pub(crate) fn span_context(trace: u8, span: u8) -> SpanContext {
    SpanContext::new(
        TraceId::from_bytes([trace; 16]),
        SpanId::from_bytes([span; 8]),
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    )
}

pub(crate) fn anchored(trace: u8, span: u8) -> AnchoredSpan {
    AnchoredSpan {
        context: span_context(trace, span),
        http_route: None,
    }
}

pub(crate) fn invalid_anchor() -> AnchoredSpan {
    AnchoredSpan {
        context: SpanContext::new(
            TraceId::INVALID,
            SpanId::INVALID,
            TraceFlags::NOT_SAMPLED,
            false,
            TraceState::default(),
        ),
        http_route: None,
    }
}
