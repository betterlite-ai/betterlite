mod body;
pub mod error;
mod items;
mod sse;
mod tests;

use std::str::from_utf8;

use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::records::{SearchObject, SearchResponse};
use indexmap::IndexMap;

use crate::agentcore_search_reply::body::parse_mcp_body;
use crate::agentcore_search_reply::error::{SearchReplyError, provider_error};
use crate::agentcore_search_reply::items::{
    structured_items, text_items, to_search_result, tool_error_detail,
};

pub fn decode_agentcore_search_reply(
    body: &[u8],
    status: u16,
    headers: &[(String, String)],
) -> Result<SearchResponse, SearchReplyError> {
    let text = from_utf8(body).map_err(|source| SearchReplyError::NotUtf8 { source })?;
    let parsed = parse_mcp_body(text, headers)?;
    let Json::Object(response) = parsed else {
        return Err(SearchReplyError::NotObject);
    };
    project(&response, status, headers)
}

fn project(
    response: &IndexMap<String, Json>,
    status: u16,
    headers: &[(String, String)],
) -> Result<SearchResponse, SearchReplyError> {
    if let Some(error) = present_error(response) {
        let rendered =
            serde_json::to_string(error).map_err(|source| SearchReplyError::Encode { source })?;
        return Err(provider_error(
            error_status(status),
            format!("AgentCore gateway MCP error: {rendered}"),
            headers,
        ));
    }
    if let Some(Json::Object(result)) = response.get("result") {
        let failed = match result.get("isError") {
            None | Some(Json::Null | Json::Bool(false)) => false,
            Some(Json::Bool(true)) => true,
            Some(Json::Number(number)) => number.as_f64().is_none_or(|value| value != 0.0_f64),
            Some(Json::Text(text)) => !text.is_empty(),
            Some(Json::List(items)) => !items.is_empty(),
            Some(Json::Object(object)) => !object.is_empty(),
        };
        if failed {
            let detail = tool_error_detail(response, result)?;
            return Err(provider_error(
                error_status(status),
                format!("AgentCore web search tool error: {detail}"),
                headers,
            ));
        }
    }
    let mut items = text_items(response);
    if items.is_empty() {
        items = structured_items(response.get("result"));
    }
    Ok(SearchResponse {
        results: items.iter().map(to_search_result).collect(),
        object: SearchObject::default(),
        extra: Object::new(),
        hidden_params: Object::new(),
    })
}

fn present_error(response: &IndexMap<String, Json>) -> Option<&Json> {
    match response.get("error") {
        None | Some(Json::Null) => None,
        Some(error) => Some(error),
    }
}

const fn error_status(status: u16) -> u16 {
    if status >= 400 { status } else { 502 }
}
