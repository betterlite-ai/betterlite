use std::sync::Mutex;
use std::time::{Duration, Instant};

use betterlite_config::sections::mock_response::selection::{MockPlan, resolve_mock};
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, StreamResponse, UpstreamResponse};
use betterlite_http::url_policy::base::ConfiguredBase;
use http::header::{ACCEPT, AUTHORIZATION};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use rand::rngs::StdRng;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value};
use thiserror::Error;
use uuid::Uuid;

use super::{client_mock, mock_model};
use crate::chat::deployment::{ChatDeployment, Provider};
use crate::chat::response::ChatResponse;
use crate::chat::{
    ChatRequest, MockChatBuilder, MockEmbeddingBuilder, MockSettings, RequestError, mock_failure,
};
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};
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
}

pub struct ChatOutcome {
    pub response: ChatResponse,
    pub upstream: UpstreamResponse,
    pub upstream_time: Duration,
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
        let outbound = self.outbound(deployment, request, "application/json")?;
        let started = Instant::now();
        let client = self
            .client
            .deployments
            .get(&deployment.model_id)
            .unwrap_or(&self.client);
        let upstream = client.send_buffered(outbound).await.map_err(http_failure)?;
        let upstream_time = started.elapsed();
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
    ) -> Result<StreamResponse, ChatError> {
        let outbound = self.outbound(deployment, request, "text/event-stream")?;
        let client = self
            .client
            .deployments
            .get(&deployment.model_id)
            .unwrap_or(&self.client);
        let response = client.send(outbound).await.map_err(http_failure)?;
        if !response.response.status().is_success() {
            let upstream = response
                .buffered(client.limits.body_bytes)
                .await
                .map_err(http_failure)?;
            return Err(ChatError::Provider {
                response: Box::new(upstream),
            });
        }
        Ok(response)
    }

    fn outbound(
        &self,
        deployment: &ChatDeployment,
        request: &ChatRequest,
        accept: &'static str,
    ) -> Result<OutboundRequest, ChatError> {
        let model = servable(deployment)?;
        let configured =
            ConfiguredBase::parse(deployment.endpoint.clone(), "litellm_params.api_base").map_err(
                |source| ChatError::Http {
                    source: TransportError::ConfiguredUrl {
                        source: Box::new(source),
                    },
                },
            )?;
        let body = serde_json::to_vec(&self.body(request, model)?)
            .map_err(|source| ChatError::Encode { source })?;
        let mut headers = HeaderMap::new();
        let credential = configured.authorization.or_else(|| {
            deployment
                .api_key
                .as_ref()
                .map(|key| SecretString::from(format!("Bearer {}", key.expose_secret())))
        });
        let authorization = credential.ok_or(ChatError::MissingApiKey)?;
        headers.insert(
            AUTHORIZATION,
            header_value(authorization.expose_secret(), "authorization")?,
        );
        headers.insert(ACCEPT, HeaderValue::from_static(accept));
        if let Some(organization) = &deployment.organization {
            headers.insert(
                HeaderName::from_static("openai-organization"),
                header_value(organization, "openai-organization")?,
            );
        }
        Ok(OutboundRequest {
            method: Method::POST,
            url: configured.url,
            query: Vec::new(),
            headers,
            body: RequestBody::Json(body),
            deadline: deployment.timeout,
        })
    }

    fn body(&self, request: &ChatRequest, model: &str) -> Result<Map<String, Value>, ChatError> {
        let Value::Object(mut requested) =
            serde_json::to_value(request).map_err(|source| ChatError::Encode { source })?
        else {
            return Ok(Map::new());
        };
        let messages = requested
            .remove("messages")
            .unwrap_or(Value::Array(Vec::new()));
        requested.remove("model");
        requested.remove("stream");
        // the mock control fields are read here and never reach the provider
        requested.remove("mock_response");
        requested.remove("mock_tool_calls");
        let (known, forwarded): (Map<String, Value>, Map<String, Value>) = requested
            .into_iter()
            .partition(|(name, _)| KNOWN_PARAMS.split(' ').any(|known| known == name));
        let policy = OpenAiConfig {
            family: ConfigFamily::OpenAi,
            api_key: None,
            headers: Headers::new(),
            settings: OpenAiSettings::default(),
        };
        let mut optional = Map::new();
        policy.map_params(model, &known, &mut optional, &self.catalog);
        if request.stream {
            optional.insert(String::from("stream"), Value::Bool(true));
        }
        if request.stream && !optional.contains_key("stream_options") {
            let mut stream_options = Map::new();
            stream_options.insert(String::from("include_usage"), Value::Bool(true));
            optional.insert(
                String::from("stream_options"),
                Value::Object(stream_options),
            );
        }
        optional.extend(forwarded);
        Ok(policy.request_body(model, messages, &optional))
    }
}

fn servable(deployment: &ChatDeployment) -> Result<&str, ChatError> {
    match &deployment.provider {
        Provider::OpenAi { model } => Ok(model),
        Provider::Unsupported { model } => Err(ChatError::UnsupportedProvider {
            model: model.clone(),
        }),
    }
}

fn header_value(text: &str, name: &str) -> Result<HeaderValue, ChatError> {
    HeaderValue::try_from(text).map_err(|source| ChatError::Http {
        source: TransportError::InvalidHeader {
            name: name.to_owned(),
            source,
        },
    })
}

mod tests;
mod transport_failure;
