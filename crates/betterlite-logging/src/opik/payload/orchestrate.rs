use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::opik::metadata::{apply_proxy_header_overrides, extract_span_identifiers, extract_tags};
use crate::opik::uuid7::create_uuid7;

use super::support::{object_field, text_field};
use super::{PayloadContent, PayloadWindow, TraceContext, build_trace_payload};

pub(super) struct RequestSections<'request> {
    pub(super) standard: &'request IndexMap<String, Json>,
    pub(super) litellm_params: &'request IndexMap<String, Json>,
    pub(super) litellm_metadata: &'request IndexMap<String, Json>,
    pub(super) standard_metadata: &'request IndexMap<String, Json>,
}

pub(super) fn request_sections<'request>(
    kwargs: &'request IndexMap<String, Json>,
    empty: &'request IndexMap<String, Json>,
) -> RequestSections<'request> {
    let standard = object_field(kwargs, "standard_logging_object").unwrap_or(empty);
    let litellm_params = object_field(kwargs, "litellm_params").unwrap_or(empty);
    let litellm_metadata = object_field(litellm_params, "metadata").unwrap_or(empty);
    let standard_metadata = object_field(standard, "metadata").unwrap_or(empty);
    RequestSections {
        standard,
        litellm_params,
        litellm_metadata,
        standard_metadata,
    }
}

pub(super) struct ResolvedContext<'request> {
    pub(super) project_name: String,
    pub(super) trace_id: String,
    pub(super) existing_trace_id: Option<String>,
    pub(super) parent_span_id: Option<String>,
    pub(super) tags: Vec<Json>,
    pub(super) thread_id: Option<String>,
    pub(super) provider_raw: Option<&'request str>,
}

pub(super) fn resolve_context<'request>(
    opik_metadata: &IndexMap<String, Json>,
    kwargs: &'request IndexMap<String, Json>,
    litellm_params: &IndexMap<String, Json>,
    project_name: &str,
) -> ResolvedContext<'request> {
    let current_span_data = opik_metadata.get("current_span_data");
    let (existing_trace_id, parent_span_id) = extract_span_identifiers(current_span_data);
    let trace_id = existing_trace_id.clone().unwrap_or_else(create_uuid7);
    let initial_project = text_field(opik_metadata, "project_name")
        .map_or_else(|| project_name.to_owned(), str::to_owned);
    let provider_raw = text_field(kwargs, "custom_llm_provider");
    let initial_tags = extract_tags(opik_metadata, provider_raw);
    let initial_thread_id = text_field(opik_metadata, "thread_id").map(str::to_owned);
    let headers = header_strings(litellm_params);
    let (resolved_project, resolved_tags, resolved_thread_id) =
        apply_proxy_header_overrides(initial_project, initial_tags, initial_thread_id, &headers);
    ResolvedContext {
        project_name: resolved_project,
        trace_id,
        existing_trace_id,
        parent_span_id,
        tags: resolved_tags,
        thread_id: resolved_thread_id,
        provider_raw,
    }
}

pub(super) fn maybe_build_trace(
    scope: &ResolvedContext<'_>,
    response: &IndexMap<String, Json>,
    window: PayloadWindow,
    body: &PayloadContent,
) -> Option<IndexMap<String, Json>> {
    if scope.existing_trace_id.is_some() {
        return None;
    }
    let trace_scope = TraceContext {
        project_name: &scope.project_name,
        trace_id: &scope.trace_id,
        response,
    };
    let trace_body = PayloadContent {
        input: body.input.clone(),
        output: body.output.clone(),
        metadata: body.metadata.clone(),
        tags: body.tags.clone(),
    };
    Some(build_trace_payload(
        &trace_scope,
        window,
        trace_body,
        scope.thread_id.clone(),
    ))
}

fn header_strings(litellm_params: &IndexMap<String, Json>) -> BTreeMap<String, String> {
    let Some(headers) = object_field(litellm_params, "proxy_server_request")
        .and_then(|request| object_field(request, "headers"))
    else {
        return BTreeMap::new();
    };
    headers
        .iter()
        .filter_map(|(key, value)| match value {
            Json::Text(text) => Some((key.clone(), text.clone())),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        })
        .collect()
}
