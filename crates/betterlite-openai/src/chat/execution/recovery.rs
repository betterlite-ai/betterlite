use std::time::{Duration, Instant};

use betterlite_http::proxy::UpstreamResponse;
use futures_util::StreamExt as _;
use futures_util::stream;
use serde_json::{Map, Value};

use super::transport_failure::http_failure;
use super::{ChatError, ChatExecutor, ChatOutcome};
use crate::chat::ChatRequest;
use crate::chat::deployment::ChatDeployment;
use crate::chat::response::ChatResponse;
use crate::output_limits::build::{RecoveredBody, Recovery, build_output_token_limit_response};
use crate::output_limits::tokens::TokenizerPromptTokens;
use crate::output_limits::{
    ErrorBody, drop_params_from_unprocessable_entity_error, is_output_token_limit_error,
};
use crate::stream::ChatStream;

impl ChatExecutor {
    // one buffered provider exchange with the time it took, which a recovered reply keeps
    pub(super) async fn buffered_attempt(
        &self,
        deployment: &ChatDeployment,
        accept: &'static str,
        body: &Map<String, Value>,
    ) -> Result<(UpstreamResponse, Duration), ChatError> {
        let outbound = self.outbound(deployment, accept, body)?;
        let client = self.client_for(deployment);
        let started = Instant::now();
        let upstream = client
            .send_following(outbound)
            .await
            .map_err(http_failure)?
            .buffered(client.limits.body_bytes)
            .await
            .map_err(http_failure)?;
        Ok((upstream, started.elapsed()))
    }

    // a 400 that matches the marker is repaired, and any other failing status travels on to the client
    pub(super) async fn buffered_outcome(
        &self,
        request: &ChatRequest,
        body: &Map<String, Value>,
        upstream: UpstreamResponse,
        upstream_time: Duration,
    ) -> Result<ChatOutcome, ChatError> {
        if upstream.status == 400 && output_limit_reached(&upstream.body) {
            return self
                .recovered_outcome(request, body, upstream, upstream_time)
                .await;
        }
        if upstream.status >= 400 {
            return Err(ChatError::Provider {
                response: Box::new(upstream),
            });
        }
        let decoded: ChatResponse = serde_json::from_slice(&upstream.body)
            .map_err(|source| ChatError::Decode { source })?;
        Ok(ChatOutcome {
            response: decoded.for_client(request.model.as_deref()),
            upstream,
            upstream_time,
        })
    }

    // LiteLLM's output-limit branch: the 400 becomes the length-truncated 200 the provider returns for larger budgets
    async fn recovered_outcome(
        &self,
        request: &ChatRequest,
        body: &Map<String, Value>,
        upstream: UpstreamResponse,
        upstream_time: Duration,
    ) -> Result<ChatOutcome, ChatError> {
        let tokens = TokenizerPromptTokens;
        let injected = Recovery {
            clock: self.clock,
            next_id: self.next_id,
            tokens: &tokens,
        };
        let recovered = build_output_token_limit_response(body, upstream.headers, &injected)
            .await
            .map_err(|source| ChatError::OutputLimit { source })?;
        let RecoveredBody::Completion(completion) = recovered.body else {
            return Err(ChatError::Streaming);
        };
        Ok(ChatOutcome {
            response: completion.for_client(request.model.as_deref()),
            upstream: UpstreamResponse {
                status: 200,
                headers: recovered.headers,
                body: Vec::new(),
            },
            upstream_time,
        })
    }

    pub(super) async fn stream_outcome(
        &self,
        body: &Map<String, Value>,
        upstream: UpstreamResponse,
    ) -> Result<ChatStream, ChatError> {
        if upstream.status == 400 && output_limit_reached(&upstream.body) {
            return self.recovered_chunk(body, upstream).await;
        }
        Err(ChatError::Provider {
            response: Box::new(upstream),
        })
    }

    // LiteLLM's streaming branch answers the one length-truncated chunk as a one-event SSE body
    async fn recovered_chunk(
        &self,
        body: &Map<String, Value>,
        upstream: UpstreamResponse,
    ) -> Result<ChatStream, ChatError> {
        let tokens = TokenizerPromptTokens;
        let injected = Recovery {
            clock: self.clock,
            next_id: self.next_id,
            tokens: &tokens,
        };
        let recovered = build_output_token_limit_response(body, upstream.headers, &injected)
            .await
            .map_err(|source| ChatError::OutputLimit { source })?;
        let RecoveredBody::Chunk(chunk) = recovered.body else {
            return Err(ChatError::Streaming);
        };
        let text = serde_json::to_string(&chunk).map_err(|source| ChatError::Encode { source })?;
        let frame = format!("data: {text}\n\n").into_bytes();
        Ok(ChatStream {
            headers: recovered.headers,
            body: stream::iter([Ok(frame)]).boxed(),
        })
    }
}

// LiteLLM re-sends a 422 once after dropping the fields the provider named, when drop_params allows it
pub(super) fn rejected_retry(
    request: &ChatRequest,
    dropped: bool,
    response: &UpstreamResponse,
    data: &Map<String, Value>,
) -> Result<Option<Map<String, Value>>, ChatError> {
    if response.status != 422 || dropped || !drop_params_enabled(request) {
        return Ok(None);
    }
    let mut retried =
        drop_params_from_unprocessable_entity_error(ErrorBody::Http(&response.body), data)
            .map_err(|source| ChatError::RejectedFields { source })?;
    // LiteLLM drops the rejected fields from the transformed body, so a retry that named model or messages would go out without them; a retry that cannot succeed is a fix none relies on, so the two required fields stay
    for name in ["messages", "model"] {
        if let Some(value) = data.get(name) {
            retried.insert(name.to_owned(), value.clone());
        }
    }
    Ok(Some(retried))
}

// the per-call drop_params kwarg; the litellm_settings and deployment defaults are not wired into the executor yet
fn drop_params_enabled(request: &ChatRequest) -> bool {
    matches!(
        request.passthrough.get("drop_params"),
        Some(Value::Bool(true))
    )
}

// LiteLLM matches the marker inside the whole exception text, which for the SDK is the raw response body
fn output_limit_reached(body: &[u8]) -> bool {
    is_output_token_limit_error(str::from_utf8(body).unwrap_or_default())
}
