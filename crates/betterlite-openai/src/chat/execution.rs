use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use betterlite_http::error::HttpError;
use betterlite_http::proxy::{JsonPost, ProxiedClient, StreamResponse, UpstreamResponse};
use rand::RngExt as _;
use rand::rngs::StdRng;
use secrecy::SecretString;
use serde_json::{Map, Value};
use thiserror::Error;

use crate::chat::deployment::{ChatDeployment, Provider};
use crate::chat::response::ChatResponse;
use crate::chat::{ChatRequest, RequestError};
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};

// a provider answer past 32 MiB is refused instead of buffered
const RESPONSE_LIMIT: usize = 0x0200_0000;

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
        source: HttpError,
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
}

impl ChatExecutor {
    pub fn pick(&self, model: &str) -> Result<&ChatDeployment, ChatError> {
        let candidates: Vec<&ChatDeployment> = self
            .deployments
            .iter()
            .filter(|deployment| deployment.model_group == model)
            .collect();
        let unknown = || ChatError::UnknownModel {
            model: model.to_owned(),
        };
        if candidates.is_empty() {
            return Err(unknown());
        }
        let index = self
            .random
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .random_range(0..candidates.len());
        candidates.get(index).copied().ok_or_else(unknown)
    }

    pub async fn send(
        &self,
        deployment: &ChatDeployment,
        request: &ChatRequest,
    ) -> Result<ChatOutcome, ChatError> {
        if request.stream {
            return Err(ChatError::Streaming);
        }
        let (model, api_key) = servable(deployment)?;
        let body = serde_json::to_vec(&self.body(request, model)?)
            .map_err(|source| ChatError::Encode { source })?;
        let started = Instant::now();
        let upstream = self
            .client
            .post_json(JsonPost {
                url: deployment.endpoint.clone(),
                bearer: api_key,
                headers: deployment
                    .organization
                    .iter()
                    .map(|organization| ("OpenAI-Organization", organization.clone()))
                    .collect(),
                body,
                timeout: deployment.timeout,
                body_limit: RESPONSE_LIMIT,
            })
            .await
            .map_err(http_failure)?;
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

    pub async fn stream(
        &self,
        deployment: &ChatDeployment,
        request: &ChatRequest,
    ) -> Result<StreamResponse, ChatError> {
        let (model, api_key) = servable(deployment)?;
        let body = serde_json::to_vec(&self.body(request, model)?)
            .map_err(|source| ChatError::Encode { source })?;
        let response = self
            .client
            .post_stream(JsonPost {
                url: deployment.endpoint.clone(),
                bearer: api_key,
                headers: deployment
                    .organization
                    .iter()
                    .map(|organization| ("OpenAI-Organization", organization.clone()))
                    .collect(),
                body,
                timeout: deployment.timeout,
                body_limit: RESPONSE_LIMIT,
            })
            .await
            .map_err(http_failure)?;
        Ok(StreamResponse::from(response))
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

fn servable(deployment: &ChatDeployment) -> Result<(&str, &SecretString), ChatError> {
    let model = match &deployment.provider {
        Provider::OpenAi { model } => model,
        Provider::Unsupported { model } => {
            return Err(ChatError::UnsupportedProvider {
                model: model.clone(),
            });
        }
    };
    let api_key = deployment
        .api_key
        .as_ref()
        .ok_or(ChatError::MissingApiKey)?;
    Ok((model, api_key))
}

fn http_failure(source: HttpError) -> ChatError {
    match source {
        HttpError::Timeout => ChatError::Timeout,
        HttpError::TooLarge { .. } => ChatError::TooLarge,
        HttpError::Required { .. }
        | HttpError::DotSegment { .. }
        | HttpError::UnsupportedProxyScheme { .. }
        | HttpError::InvalidProxyUrl { .. }
        | HttpError::Client(_)
        | HttpError::Request(_) => ChatError::Http { source },
    }
}

mod tests;
