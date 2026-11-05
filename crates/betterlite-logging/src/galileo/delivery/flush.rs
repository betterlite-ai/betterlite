use std::sync::PoisonError;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::auth::ensure_headers;
use super::headers::redact_headers;
use super::transport::{HttpResponse, TraceIds, Transport};
use super::{GalileoDelivery, Record, diagnostics};
use crate::galileo_payloads::trace::build_traces_payload;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlushOutcome {
    Empty,
    NotConfigured,
    NoHeaders,
    Delivered,
    Rejected,
    TransportFailed,
}

// GalileoObserve._get_ingest_request over a snapshot of the records, so appends made during the request stay out of this payload.
pub fn ingest_request(
    delivery: &GalileoDelivery,
    records: &[Record],
    ids: &dyn TraceIds,
) -> Option<(String, IndexMap<String, Json>)> {
    let config = &delivery.config;
    let (Some(base_url), Some(project_id)) = (&config.base_url, &config.project_id) else {
        return None;
    };
    let trace_span_ids: Vec<(String, String)> = records
        .iter()
        .map(|_| (ids.next_id(), ids.next_id()))
        .collect();
    let payload = build_traces_payload(records, &trace_span_ids, config.log_stream_id.as_deref());
    if config.use_v2_api {
        return Some((format!("{base_url}/ingest/traces/{project_id}"), payload));
    }
    Some((
        format!("{base_url}/v2/projects/{project_id}/traces"),
        payload,
    ))
}

// GalileoObserve.flush_in_memory_records.
pub fn deliver(
    delivery: &GalileoDelivery,
    transport: &dyn Transport,
    ids: &dyn TraceIds,
) -> FlushOutcome {
    let records = delivery
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .records
        .clone();
    if records.is_empty() {
        return FlushOutcome::Empty;
    }
    let Some((url, payload)) = ingest_request(delivery, &records, ids) else {
        tracing::debug!(
            "Galileo Logger: missing GALILEO_BASE_URL or GALILEO_PROJECT_ID, skipping flush"
        );
        return FlushOutcome::NotConfigured;
    };
    if !ensure_headers(delivery, transport) {
        tracing::debug!("Galileo Logger: could not set request headers, skipping flush");
        return FlushOutcome::NoHeaders;
    }
    let headers = delivery
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .headers
        .clone()
        .unwrap_or_default();
    log_flush_config(delivery, records.len());
    diagnostics::log_flush_payload(delivery.config.use_v2_api, &url, &payload);
    tracing::debug!(
        "Galileo Logger flush headers: {:?}",
        redact_headers(&headers)
    );
    tracing::debug!("flushing in memory records to {url}");
    match transport.post_json(&url, &headers, &payload) {
        Ok(response) => settle(delivery, &response, records.len()),
        Err(error) => {
            tracing::debug!("Galileo Logger: failed to flush in memory records: {error}");
            FlushOutcome::TransportFailed
        }
    }
}

// Only the captured prefix goes on success; a token-auth rejection drops the cached headers so the next flush logs in again.
fn settle(delivery: &GalileoDelivery, response: &HttpResponse, captured: usize) -> FlushOutcome {
    let mut state = delivery
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if (200..300).contains(&response.status) {
        tracing::debug!("Galileo Logger: successfully flushed in memory records");
        tracing::debug!(
            "Galileo Logger flush response: status={} body={}",
            response.status,
            response.body
        );
        let sent = captured.min(state.records.len());
        state.records.drain(..sent);
        return FlushOutcome::Delivered;
    }
    tracing::debug!("Galileo Logger: failed to flush in memory records");
    tracing::debug!(
        "Galileo Logger error={}, status code={}",
        response.body,
        response.status
    );
    if !delivery.config.use_v2_api && matches!(response.status, 401 | 403) {
        state.headers = None;
    }
    FlushOutcome::Rejected
}

// GalileoObserve._log_flush_config.
fn log_flush_config(delivery: &GalileoDelivery, record_count: usize) {
    let config = &delivery.config;
    tracing::debug!(
        "Galileo Logger flush config: use_v2_api={} base_url={:?} project_id={:?} log_stream_id={:?} api_key_set={} username_set={} record_count={record_count}",
        config.use_v2_api,
        config.base_url,
        config.project_id,
        config.log_stream_id,
        config.api_key.is_some(),
        config.username.is_some(),
    );
}
