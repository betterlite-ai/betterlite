use betterlite_models::decisions::Json;

use crate::agentcore_search_reply::error::{SearchReplyError, provider_error};
use crate::agentcore_search_reply::sse::{iter_sse_events, looks_like_sse};

pub(crate) fn parse_mcp_body(
    text: &str,
    headers: &[(String, String)],
) -> Result<Json, SearchReplyError> {
    if !looks_like_sse(text) {
        let json = text.strip_prefix('\u{feff}').unwrap_or(text);
        return serde_json::from_str::<Json>(json)
            .map_err(|source| SearchReplyError::Json { source });
    }
    let mut events = iter_sse_events(text);
    if let Some(event) = events.iter().find(|event| carries_response(event)) {
        return Ok(event.clone());
    }
    if let Some(last) = events.pop() {
        return Ok(last);
    }
    let preview: String = text.chars().take(200).collect();
    Err(provider_error(
        502,
        format!("AgentCore gateway returned SSE without a JSON data frame: {preview}"),
        headers,
    ))
}

fn carries_response(event: &Json) -> bool {
    let Json::Object(fields) = event else {
        return false;
    };
    fields.contains_key("result") || fields.contains_key("error")
}
