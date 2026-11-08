use std::iter::once;

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::value::RawValue;

use super::hook_kwargs::{HookMetadata, ParentOtelSpan, SearchCaller, SearchHookKwargs};
use super::search_metadata::{SearchMetadataValue, SearchRequestMetadata};

#[derive(Debug, Clone, Default, Serialize)]
pub struct ParentRequestCorrelation {
    pub session_id: Option<String>,
    pub trace_id: Option<String>,
    pub parent_request_id: Option<String>,
    pub parent_otel_span: Option<ParentOtelSpan>,
}

impl ParentRequestCorrelation {
    #[must_use]
    pub fn as_search_metadata(&self) -> SearchRequestMetadata {
        let mut entries = SearchRequestMetadata::new();
        if let Some(session_id) = &self.session_id {
            entries.insert(
                "session_id".to_owned(),
                SearchMetadataValue::Text(session_id.clone()),
            );
        }
        if let Some(trace_id) = &self.trace_id {
            entries.insert(
                "trace_id".to_owned(),
                SearchMetadataValue::Text(trace_id.clone()),
            );
        }
        if let Some(parent_request_id) = &self.parent_request_id {
            entries.insert(
                "parent_request_id".to_owned(),
                SearchMetadataValue::Text(parent_request_id.clone()),
            );
        }
        if let Some(span) = &self.parent_otel_span {
            entries.insert(
                "litellm_parent_otel_span".to_owned(),
                SearchMetadataValue::Span(span.clone()),
            );
        }
        entries
    }

    #[must_use]
    pub fn as_search_kwargs(&self) -> IndexMap<String, String> {
        [
            ("litellm_session_id", &self.session_id),
            ("litellm_trace_id", &self.trace_id),
        ]
        .into_iter()
        .filter_map(|(key, value)| value.clone().map(|text| (key.to_owned(), text)))
        .collect()
    }
}

#[must_use]
pub fn parent_correlation(
    kwargs: Option<&SearchHookKwargs>,
    caller: Option<&SearchCaller>,
) -> ParentRequestCorrelation {
    let Some(hook_kwargs) = kwargs else {
        return ParentRequestCorrelation::default();
    };
    if hook_kwargs.is_empty() {
        return ParentRequestCorrelation::default();
    }
    let params = hook_kwargs.litellm_params.as_ref();
    let metadatas = metadata_blocks(hook_kwargs);
    let session_id = once(hook_kwargs.litellm_session_id.as_deref())
        .chain(params.map(|scoped| scoped.litellm_session_id.as_deref()))
        .chain(
            metadatas
                .iter()
                .map(|metadata| metadata.session_id.as_deref()),
        )
        .find_map(json_string);
    let trace_id = once(hook_kwargs.litellm_trace_id.as_deref())
        .chain(params.map(|scoped| scoped.litellm_trace_id.as_deref()))
        .chain(
            metadatas
                .iter()
                .map(|metadata| metadata.trace_id.as_deref()),
        )
        .find_map(json_string);
    let parent_request_id = once(hook_kwargs.litellm_call_id.as_deref())
        .chain(params.map(|scoped| scoped.litellm_call_id.as_deref()))
        .find_map(json_string);
    ParentRequestCorrelation {
        session_id,
        trace_id,
        parent_request_id,
        parent_otel_span: first_parent_span(&metadatas, caller),
    }
}

fn metadata_blocks(hook_kwargs: &SearchHookKwargs) -> Vec<&HookMetadata> {
    let mut metadatas: Vec<&HookMetadata> = [
        hook_kwargs.metadata.as_ref(),
        hook_kwargs.litellm_metadata.as_ref(),
    ]
    .into_iter()
    .flatten()
    .collect();
    if let Some(params) = &hook_kwargs.litellm_params {
        metadatas.extend(params.metadata.iter());
        metadatas.extend(params.litellm_metadata.iter());
    }
    metadatas
}

fn first_parent_span(
    metadatas: &[&HookMetadata],
    caller: Option<&SearchCaller>,
) -> Option<ParentOtelSpan> {
    metadatas
        .iter()
        .find_map(|metadata| metadata.litellm_parent_otel_span.clone())
        .or_else(|| {
            let auth = caller?;
            auth.parent_otel_span.clone()
        })
}

fn json_string(value: Option<&RawValue>) -> Option<String> {
    let text = serde_json::from_str::<String>(value?.get()).ok()?;
    if text.is_empty() {
        return None;
    }
    Some(text)
}
