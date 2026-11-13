use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::decisions::Json;

// types/utils.py's ServerToolUse: each counter is optional, so an unreported count stays distinct from a reported zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct ServerToolUse {
    #[serde(default)]
    pub web_search_requests: Option<i64>,
    #[serde(default)]
    pub tool_search_requests: Option<i64>,
    #[serde(default)]
    pub browser_open_requests: Option<i64>,
}

// CostUsage is built by many crates, so the parsed fields the Anthropic calculations read travel beside it instead of changing its fields.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct ServerToolProjection {
    #[serde(default)]
    pub server_tool_use: Option<ServerToolUse>,
    #[serde(default)]
    pub inference_geo: Option<String>,
    #[serde(default)]
    pub speed: Option<String>,
}

// A parsed ServerToolUse object, or the plain mapping streaming_chunk_builder can leave on a usage chunk.
#[derive(Debug, Clone, Copy)]
pub enum ServerToolRecord<'record> {
    Parsed(&'record ServerToolUse),
    Mapping(&'record IndexMap<String, Json>),
}

// utils.py get_web_search_requests: an absent counter is None, a reported zero stays zero, and a mapping value is read without validation.
#[must_use]
pub fn web_search_requests(record: Option<ServerToolRecord<'_>>) -> Option<Json> {
    match record? {
        ServerToolRecord::Parsed(usage) => usage
            .web_search_requests
            .map(|count| Json::Number(serde_json::Number::from(count))),
        ServerToolRecord::Mapping(mapping) => match mapping.get("web_search_requests") {
            None | Some(Json::Null) => None,
            Some(value) => Some(value.clone()),
        },
    }
}

mod tests;
