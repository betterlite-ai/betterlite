mod orchestrate;
mod support;

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;

use super::config::create_usage_object;
use super::metadata::{extract_and_build_metadata, extract_opik_metadata, normalize_provider_name};

use orchestrate::{
    RequestSections, ResolvedContext, maybe_build_trace, request_sections, resolve_context,
};
use support::{
    common_payload, extend_payload, remove_nulls, span_entries, span_identity, string_or,
};

type PayloadList = Vec<IndexMap<String, Json>>;

#[derive(Clone, Copy)]
pub struct PayloadWindow {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

pub struct PayloadContent {
    pub input: Json,
    pub output: Json,
    pub metadata: IndexMap<String, Json>,
    pub tags: Vec<Json>,
}

pub struct TraceContext<'request> {
    pub project_name: &'request str,
    pub trace_id: &'request str,
    pub response: &'request IndexMap<String, Json>,
}

pub struct SpanContext<'request> {
    pub project_name: &'request str,
    pub trace_id: &'request str,
    pub parent_span_id: Option<String>,
    pub response: &'request IndexMap<String, Json>,
}

pub struct SpanDetails<'request> {
    pub cost: Option<Json>,
    pub provider: Option<&'request str>,
}

#[must_use]
pub fn build_trace_payload(
    scope: &TraceContext<'_>,
    window: PayloadWindow,
    body: PayloadContent,
    thread_id: Option<String>,
) -> IndexMap<String, Json> {
    let mut payload = common_payload(scope.project_name, window, body);
    extend_payload(
        &mut payload,
        [
            ("id", Json::Text(scope.trace_id.to_owned())),
            (
                "name",
                Json::Text(string_or(scope.response, "object", "unknown type")),
            ),
        ],
    );
    if let Some(id) = thread_id {
        payload.insert("thread_id".to_owned(), Json::Text(id));
    }
    remove_nulls(payload)
}

#[must_use]
pub fn build_span_payload(
    scope: SpanContext<'_>,
    window: PayloadWindow,
    body: PayloadContent,
    details: SpanDetails<'_>,
) -> IndexMap<String, Json> {
    let identity = span_identity(scope.response);
    let usage = match scope.response.get("usage") {
        Some(Json::Object(value)) => create_usage_object(value),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => IndexMap::new(),
    };
    let mut payload = common_payload(scope.project_name, window, body);
    extend_payload(
        &mut payload,
        span_entries(scope, window, identity, details, usage),
    );
    remove_nulls(payload)
}

#[must_use]
pub fn build_opik_payload(
    kwargs: &IndexMap<String, Json>,
    response: &IndexMap<String, Json>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    project_name: &str,
) -> (Option<IndexMap<String, Json>>, IndexMap<String, Json>) {
    let empty = IndexMap::new();
    let sections = request_sections(kwargs, &empty);
    let opik_metadata =
        extract_opik_metadata(sections.litellm_metadata, sections.standard_metadata);
    let scope = resolve_context(
        &opik_metadata,
        kwargs,
        sections.litellm_params,
        project_name,
    );
    let body = payload_content(&sections, &opik_metadata, &scope, kwargs);
    let window = PayloadWindow { start, end };
    let trace = maybe_build_trace(&scope, response, window, &body);
    let span = build_span_from_scope(&scope, kwargs, response, window, body);
    (trace, span)
}

fn payload_content(
    sections: &RequestSections<'_>,
    opik_metadata: &IndexMap<String, Json>,
    scope: &ResolvedContext<'_>,
    kwargs: &IndexMap<String, Json>,
) -> PayloadContent {
    let metadata = extract_and_build_metadata(
        opik_metadata,
        sections.standard_metadata,
        sections.standard,
        kwargs,
    );
    PayloadContent {
        input: sections
            .standard
            .get("messages")
            .cloned()
            .unwrap_or_else(|| Json::Object(IndexMap::new())),
        output: sections
            .standard
            .get("response")
            .cloned()
            .unwrap_or_else(|| Json::Object(IndexMap::new())),
        metadata,
        tags: scope.tags.clone(),
    }
}

fn build_span_from_scope(
    scope: &ResolvedContext<'_>,
    kwargs: &IndexMap<String, Json>,
    response: &IndexMap<String, Json>,
    window: PayloadWindow,
    body: PayloadContent,
) -> IndexMap<String, Json> {
    let provider = normalize_provider_name(scope.provider_raw);
    let span_context = SpanContext {
        project_name: &scope.project_name,
        trace_id: &scope.trace_id,
        parent_span_id: scope.parent_span_id.clone(),
        response,
    };
    let span_details = SpanDetails {
        cost: kwargs.get("response_cost").cloned(),
        provider: provider.as_deref(),
    };
    build_span_payload(span_context, window, body, span_details)
}

#[must_use]
pub fn classify_payload(payload: &[IndexMap<String, Json>]) -> (PayloadList, PayloadList) {
    let traces = payload
        .iter()
        .filter(|item| !item.contains_key("type"))
        .cloned()
        .map(remove_nulls)
        .collect();
    let spans = payload
        .iter()
        .filter(|item| item.contains_key("type"))
        .cloned()
        .map(remove_nulls)
        .collect();
    (traces, spans)
}
