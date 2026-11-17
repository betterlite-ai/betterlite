use std::sync::Mutex;
use std::time::{Duration, Instant};

use betterlite_config::sections::mock_response::selection::{MockPlan, resolve_mock};
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_redact::secret::Redactor;
use futures_util::stream::BoxStream;
use http::HeaderMap;
use rand::rngs::StdRng;
use serde_json::{Map, Value};
use thiserror::Error;
use uuid::Uuid;

use super::{client_mock, mock_model};
use crate::chat::deployment::{ChatDeployment, Provider};
use crate::chat::response::ChatResponse;
use crate::chat::{
    ChatRequest, MockChatBuilder, MockEmbeddingBuilder, MockSettings, RequestError, mock_failure,
};
use crate::openai_base_config::CatalogCapabilities;
use crate::output_limits::RejectedFieldsError;
use crate::output_limits::tokens::PromptCountError;
use outbound::outbound_with;
use recovery::rejected_retry;
use transport_failure::http_failure;

// LiteLLM's DEFAULT_CHAT_COMPLETION_PARAM_VALUES: these go through the OpenAI parameter policy, the rest are forwarded
const KNOWN_PARAMS: &str = "functions function_call temperature top_p n stream stream_options stop max_tokens max_completion_tokens modalities prediction audio presence_penalty frequency_penalty logit_bias user model custom_llm_provider response_format seed tools tool_choice max_retries logprobs top_logprobs extra_headers api_version parallel_tool_calls drop_params allowed_openai_params additional_drop_params messages reasoning_effort verbosity thinking web_search_options include_server_side_tool_invocations service_tier safety_identifier prompt_cache_key prompt_cache_retention store metadata context_management";

#[derive(Debug, Error)]
pub enum ChatError {
    #[error("no model_list entry is named {model}")]
    UnknownModel { model: String },
    #[error(transparent)]
    Request {
        #[from]
        source: RequestError,
    },
    #[error("betterlite does not support streaming through the buffered chat completion path")]
    Streaming,
    #[error("betterlite does not serve {model} chat completions yet")]
    UnsupportedProvider { model: String },
    #[error("no API key is set for this deployment, in litellm_settings or in OPENAI_API_KEY")]
    MissingApiKey,
    #[error("the provider did not answer within the deployment timeout")]
    Timeout,
    #[error("the provider answered with status {}", .response.status)]
    Provider { response: Box<UpstreamResponse> },
    #[error("the request could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the provider answer is not a chat completion: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the provider answer is larger than betterlite accepts")]
    TooLarge,
    #[error("the provider could not be reached: {source}")]
    Http {
        #[source]
        source: TransportError,
    },
    #[error("the prompt of an output-limit reply could not be counted: {source}")]
    OutputLimit {
        #[source]
        source: PromptCountError,
    },
    #[error("the rejected fields of the provider's 422 could not be read: {source}")]
    RejectedFields {
        #[source]
        source: RejectedFieldsError,
    },
}

pub struct ChatOutcome {
    pub response: ChatResponse,
    pub upstream: UpstreamResponse,
    pub upstream_time: Duration,
}

// the chunks of a provider stream with the upstream response headers the client receives alongside them
pub struct ChatStream {
    pub headers: HeaderMap,
    pub body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
}

pub struct ChatExecutor {
    pub client: ProxiedClient,
    pub deployments: Vec<ChatDeployment>,
    pub catalog: CatalogCapabilities,
    pub random: Mutex<StdRng>,
    pub clock: fn() -> u64,
    pub next_id: fn() -> Uuid,
    pub mock: MockSettings,
    pub mock_chat: MockChatBuilder,
    pub mock_embedding: MockEmbeddingBuilder,
    // built once at startup and shared by every request that renders a provider error
    pub redactor: Redactor,
}

impl ChatExecutor {
    pub async fn send(
        &self,
        deployment: &ChatDeployment,
        request: &ChatRequest,
    ) -> Result<ChatOutcome, ChatError> {
        if request.stream {
            return Err(ChatError::Streaming);
        }
        let (client_response, client_tool_calls) =
            client_mock(&request.passthrough, self.mock.permission)
                .map_err(|source| ChatError::from(RequestError::Body { source }))?;
        let resolved = resolve_mock(
            self.mock.responses.get(&deployment.model_id),
            client_response.as_ref(),
            client_tool_calls.as_deref(),
            request.choice_count,
        );
        if let Some(plan) = resolved {
            return self.mocked_outcome(&plan, deployment, request);
        }
        let model = servable(deployment)?;
        let mut body = self.body(request, model)?;
        let mut dropped = false;
        loop {
            let upstream = self.buffered(deployment, &body).await?;
            if let Some(retried) = rejected_retry(request, dropped, &upstream.response, &body)? {
                body = retried;
                dropped = true;
                continue;
            }
            return self.buffered_outcome(request, &body, upstream).await;
        }
    }

    async fn buffered(
        &self,
        deployment: &ChatDeployment,
        body: &Map<String, Value>,
    ) -> Result<Upstream, ChatError> {
        let client = self.client_for(deployment);
        let outbound = outbound_with(deployment, "application/json", body)?;
        let started = Instant::now();
        let response = client
            .send_following(outbound)
            .await
            .map_err(http_failure)?;
        let response = response
            .buffered(client.limits.body_bytes)
            .await
            .map_err(http_failure)?;
        Ok(Upstream {
            response,
            time: started.elapsed(),
        })
    }

    fn mocked_outcome(
        &self,
        plan: &MockPlan<'_>,
        deployment: &ChatDeployment,
        request: &ChatRequest,
    ) -> Result<ChatOutcome, ChatError> {
        let id = format!("chatcmpl-{}", (self.next_id)());
        let response = (self.mock_chat)(
            plan,
            self.mock.usage,
            mock_model(&deployment.litellm_model),
            &id,
            (self.clock)(),
        )
        .map_err(mock_failure)?;
        Ok(ChatOutcome {
            response: response.for_client(request.model.as_deref()),
            upstream: UpstreamResponse {
                status: 200,
                headers: HeaderMap::new(),
                body: Vec::new(),
            },
            upstream_time: Duration::ZERO,
        })
    }

    pub async fn stream(
        &self,
        deployment: &ChatDeployment,
        request: &ChatRequest,
    ) -> Result<ChatStream, ChatError> {
        let model = servable(deployment)?;
        let mut body = self.body(request, model)?;
        let mut dropped = false;
        loop {
            let client = self.client_for(deployment);
            let outbound = outbound_with(deployment, "text/event-stream", &body)?;
            let response = client
                .send_following(outbound)
                .await
                .map_err(http_failure)?;
            if response.response.status().is_success() {
                let headers = response.response.headers().clone();
                return Ok(ChatStream {
                    headers,
                    body: response.body_stream(),
                });
            }
            let upstream = response
                .buffered(client.limits.body_bytes)
                .await
                .map_err(http_failure)?;
            if let Some(retried) = rejected_retry(request, dropped, &upstream, &body)? {
                body = retried;
                dropped = true;
                continue;
            }
            return self.stream_outcome(&body, upstream).await;
        }
    }
}

// the buffered provider answer with the time it took, which a recovered reply keeps
struct Upstream {
    response: UpstreamResponse,
    time: Duration,
}

fn servable(deployment: &ChatDeployment) -> Result<&str, ChatError> {
    match &deployment.provider {
        Provider::OpenAi { model } => Ok(model),
        Provider::Unsupported { model } => Err(ChatError::UnsupportedProvider {
            model: model.clone(),
        }),
    }
}

mod outbound;
mod recovery;
mod tests;
mod transport_failure;
