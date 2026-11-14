use std::error::Error;

use betterlite_core::agents::json::AgentJson;
use futures_core::Stream;
use futures_util::StreamExt as _;

use super::call::StreamedEvents;
use super::runtime::{StreamedResponse, collect_body};
use super::{AgentCoreError, TransformError};

const JSON_CONTENT_TYPE: &str = "application/json";

#[must_use]
pub fn parse_sse_events<L, E>(lines: L) -> impl Stream<Item = Result<AgentJson, AgentCoreError<E>>>
where
    L: Stream<Item = Result<String, E>>,
    E: Error + 'static,
{
    async_stream::stream! {
        let mut pinned = std::pin::pin!(lines);
        while let Some(next) = pinned.next().await {
            let raw = match next {
                Ok(raw) => raw,
                Err(source) => {
                    yield Err(AgentCoreError::Runtime(source));
                    return;
                }
            };
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Some(rest) = trimmed.strip_prefix("data:") else {
                continue;
            };
            let payload = rest.trim();
            if payload.is_empty() {
                continue;
            }
            match serde_json::from_str::<AgentJson>(payload) {
                Ok(event) => yield Ok(event),
                Err(_) => tracing::debug!("BedrockAgentCore A2A: skipping a non-JSON SSE line"),
            }
        }
    }
}

pub(super) fn decode_body(body: &[u8]) -> Result<AgentJson, TransformError> {
    let value: AgentJson = serde_json::from_slice(body).map_err(TransformError::Decode)?;
    if value.get("error").is_some() {
        tracing::warn!("BedrockAgentCore A2A: the agent returned an error response");
    }
    Ok(value)
}

pub(super) fn replies<L, E>(response: StreamedResponse<L>) -> StreamedEvents<E>
where
    L: Stream<Item = Result<String, E>> + Send + 'static,
    E: Error + Send + 'static,
{
    let json_reply = response
        .content_type
        .to_lowercase()
        .contains(JSON_CONTENT_TYPE);
    let lines = response.lines;
    Box::pin(async_stream::stream! {
        if json_reply {
            let body = match collect_body(lines).await {
                Ok(body) => body,
                Err(error) => {
                    yield Err(error);
                    return;
                }
            };
            match serde_json::from_str::<AgentJson>(&body) {
                Ok(value) => yield Ok(value),
                Err(source) => yield Err(AgentCoreError::Transform(TransformError::Decode(source))),
            }
        } else {
            let mut events = std::pin::pin!(parse_sse_events(lines));
            while let Some(event) = events.next().await {
                yield event;
            }
        }
    })
}
