use betterlite_models::decisions::Json;
use betterlite_openai::search::records::SearchResponse;

use crate::agentcore_search_reply::decode_agentcore_search_reply;
use crate::agentcore_search_reply::error::SearchReplyError;
use crate::bedrock_settings::{BedrockError, ErrorHeaders};

pub(super) fn headers() -> Vec<(String, String)> {
    vec![
        (
            String::from("content-type"),
            String::from("application/json"),
        ),
        (String::from("x-request-id"), String::from("req-1")),
    ]
}

pub(super) fn decode(body: &str, status: u16) -> Result<SearchResponse, SearchReplyError> {
    decode_with(body, status, &headers())
}

pub(super) fn decode_bytes(bytes: &[u8], status: u16) -> Result<SearchResponse, SearchReplyError> {
    decode_agentcore_search_reply(bytes, status, &headers())
}

pub(super) fn decode_with(
    body: &str,
    status: u16,
    headers: &[(String, String)],
) -> Result<SearchResponse, SearchReplyError> {
    decode_agentcore_search_reply(body.as_bytes(), status, headers)
}

pub(super) fn titles(response: &SearchResponse) -> Vec<&str> {
    response
        .results
        .iter()
        .map(|result| result.title.as_str())
        .collect()
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn provider_status(error: &SearchReplyError) -> u16 {
    provider(error).error.status_code
}

pub(super) fn provider_message(error: &SearchReplyError) -> &str {
    provider(error).error.message.as_str()
}

pub(super) fn provider_headers(error: &SearchReplyError) -> Option<&ErrorHeaders> {
    provider(error).headers.as_ref()
}

fn provider(error: &SearchReplyError) -> &BedrockError {
    let SearchReplyError::Provider { cause, .. } = error else {
        panic!("expected a provider error, found {error}");
    };
    cause.cause.as_ref()
}
