use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::records::{SearchObject, SearchResponse, SearchResult};
use serde::Deserialize;
use thiserror::Error;

use super::request::PARALLEL_AI_USAGE_PARAM;

#[derive(Debug, Error)]
pub enum ParallelAiReplyError {
    #[error("the Parallel AI search reply does not match the v1 search schema: {source}")]
    Schema {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Deserialize)]
struct ParallelReply {
    #[serde(default)]
    search_id: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    results: Vec<ParallelResult>,
    #[serde(default)]
    usage: Option<Vec<Object>>,
    #[serde(default)]
    warnings: Option<Vec<Object>>,
    // a flattened map keeps serde from accepting a JSON array in place of this object
    #[serde(flatten)]
    _extra: Object,
}

#[derive(Debug, Deserialize)]
struct ParallelResult {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    publish_date: Option<String>,
    #[serde(default)]
    excerpts: Option<Vec<String>>,
    // a flattened map keeps serde from accepting a JSON array in place of this object
    #[serde(flatten)]
    _extra: Object,
}

pub fn decode_parallel_ai_reply(bytes: &[u8]) -> Result<SearchResponse, ParallelAiReplyError> {
    let reply: ParallelReply =
        serde_json::from_slice(bytes).map_err(|source| ParallelAiReplyError::Schema { source })?;
    let usage = usage_value(reply.usage.as_ref());
    let mut extra = Object::new();
    if let Some(search_id) = reply.search_id {
        extra.insert("search_id".to_owned(), JsonValue::String(search_id));
    }
    if let Some(session_id) = reply.session_id {
        extra.insert("session_id".to_owned(), JsonValue::String(session_id));
    }
    // the reserved `usage` name stays free for LiteLLM's own token usage object
    if reply.usage.is_some() {
        extra.insert("parallel_usage".to_owned(), usage.clone());
    }
    if let Some(warnings) = reply.warnings {
        extra.insert(
            "warnings".to_owned(),
            JsonValue::Array(warnings.into_iter().map(JsonValue::Object).collect()),
        );
    }
    // written even when null, so a caller-supplied usage can never price its own request
    let mut hidden_params = Object::new();
    hidden_params.insert(PARALLEL_AI_USAGE_PARAM.to_owned(), usage);
    Ok(SearchResponse {
        results: reply.results.into_iter().map(project).collect(),
        object: SearchObject::default(),
        extra,
        hidden_params,
    })
}

fn usage_value(usage: Option<&Vec<Object>>) -> JsonValue {
    match usage {
        Some(entries) => JsonValue::Array(
            entries
                .iter()
                .cloned()
                .map(JsonValue::Object)
                .collect::<Vec<_>>(),
        ),
        None => JsonValue::Null,
    }
}

fn project(result: ParallelResult) -> SearchResult {
    let excerpts = result.excerpts.unwrap_or_default();
    let snippet = excerpts.join(" ... ");
    let mut extra = Object::new();
    extra.insert(
        "excerpts".to_owned(),
        JsonValue::Array(excerpts.into_iter().map(JsonValue::String).collect()),
    );
    SearchResult {
        title: result.title.unwrap_or_default(),
        url: result.url.unwrap_or_default(),
        snippet,
        date: result.publish_date,
        last_updated: None,
        extra,
    }
}
