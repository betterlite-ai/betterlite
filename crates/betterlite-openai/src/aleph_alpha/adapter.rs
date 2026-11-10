use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::{ProxiedClient, StreamResponse};
use betterlite_tokenizer::engine::Engine;
use http::Method;
use secrecy::SecretString;

use crate::aleph_alpha::error::AlephAlphaError;
use crate::aleph_alpha::lines::{LineStream, decode_lines, decode_text};
use crate::aleph_alpha::reply::{decode_aleph_reply, resolve_usage};
use crate::aleph_alpha::request::{CompletionInput, CompletionRequest, completion_request};
use crate::chat::response::{ChatChoice, ChatUsage};
use crate::openai_base_config::Object;

pub struct AlephAlphaAdapter {
    pub client: ProxiedClient,
    pub tokenizer: Engine,
    pub timeout: Duration,
}

#[derive(Debug)]
pub struct CompletionReply {
    pub choices: Vec<ChatChoice>,
    pub created: i64,
    pub model: String,
    pub usage: ChatUsage,
}

pub enum CompletionOutcome {
    Reply(Box<CompletionReply>),
    Stream(LineStream),
}

// LiteLLM's logging_obj: pre_call before the request, post_call after a buffered reply and never for a stream
pub enum CallPhase<'call> {
    Pre {
        prompt: &'call str,
        api_key: Option<&'call SecretString>,
        body: &'call Object,
    },
    Post {
        prompt: &'call str,
        api_key: Option<&'call SecretString>,
        body: &'call Object,
        response_text: &'call str,
    },
}

impl AlephAlphaAdapter {
    pub async fn completion(
        &self,
        input: &CompletionInput<'_>,
        log: &mut (dyn FnMut(CallPhase<'_>) + Send),
    ) -> Result<CompletionOutcome, AlephAlphaError> {
        let request = completion_request(input)?;
        log(CallPhase::Pre {
            prompt: &request.prompt,
            api_key: input.api_key,
            body: &request.body,
        });
        let response = self.send(&request).await?;
        let limit = self.client.limits.body_bytes;
        // HTTPHandler.post raises on any non-2xx status before the stream check or any parsing
        if !response.response.status().is_success() {
            return Err(status_error(response, limit).await);
        }
        if request.stream {
            return Ok(CompletionOutcome::Stream(decode_lines(response, limit)));
        }
        let upstream = response
            .buffered(limit)
            .await
            .map_err(|source| AlephAlphaError::Transport { source })?;
        let text = decode_text(&upstream.body);
        log(CallPhase::Post {
            prompt: &request.prompt,
            api_key: input.api_key,
            body: &request.body,
            response_text: &text,
        });
        let choices = decode_aleph_reply(upstream.status, &upstream.body)?;
        let usage = resolve_usage(&request.prompt, &choices, &self.tokenizer)?;
        Ok(CompletionOutcome::Reply(Box::new(CompletionReply {
            choices,
            created: input.created,
            model: input.model.to_owned(),
            usage,
        })))
    }

    async fn send(&self, request: &CompletionRequest) -> Result<StreamResponse, AlephAlphaError> {
        let body = serde_json::to_vec(&request.body)
            .map_err(|source| AlephAlphaError::Encode { source })?;
        let outbound = OutboundRequest {
            method: Method::POST,
            url: request.url.clone(),
            query: Vec::new(),
            headers: request.headers.clone(),
            body: RequestBody::Json(body),
            deadline: self.timeout,
        };
        self.client
            .send(outbound)
            .await
            .map_err(|source| AlephAlphaError::Transport { source })
    }
}

async fn status_error(response: StreamResponse, limit: usize) -> AlephAlphaError {
    match response.buffered(limit).await {
        Ok(failed) => AlephAlphaError::Provider {
            status_code: failed.status,
            message: mask_sensitive_info(&decode_text(&failed.body)),
        },
        Err(source) => AlephAlphaError::Transport { source },
    }
}

// LiteLLM's http_handler.mask_sensitive_info: the first key= value goes, up to the next & or the end
#[must_use]
pub fn mask_sensitive_info(text: &str) -> String {
    let Some(found) = text.find("key=") else {
        return text.to_owned();
    };
    let value_start = found.saturating_add(4);
    let rest = text.get(value_start..).unwrap_or_default();
    let tail = rest
        .find('&')
        .and_then(|next| rest.get(next..))
        .unwrap_or_default();
    let head = text.get(..value_start).unwrap_or_default();
    format!("{head}[REDACTED_API_KEY]{tail}")
}
