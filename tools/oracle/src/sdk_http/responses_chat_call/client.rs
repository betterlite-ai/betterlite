use std::collections::BTreeMap;
use std::future::Future;
use std::str::FromStr as _;
use std::sync::{Mutex, PoisonError};

use betterlite_core::api::responses_chat_call::completions::{
    CompletionArguments, CompletionBody, CompletionClient, CompletionOutput,
};
use betterlite_core::api::responses_chat_output::records::ChatCompletionResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::{ProxiedClient, StreamResponse, UpstreamResponse};
use betterlite_openai::chat::deployment::ChatDeployment;
use http::Method;
use http::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use secrecy::ExposeSecret as _;
use serde_json::{Map, Value};
use thiserror::Error;

use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::Failure;
use crate::sdk_http::{ChatMessage, OpenaiChat, native};

use super::call::Call;

// The OpenAI chat body parameters the Responses transform can map; every litellm-only argument stays out
const BODY_PARAMS: &[&str] = &[
    "context_management",
    "max_tokens",
    "messages",
    "metadata",
    "model",
    "parallel_tool_calls",
    "reasoning_effort",
    "reasoning_summary",
    "response_format",
    "safety_identifier",
    "service_tier",
    "stream",
    "stream_options",
    "temperature",
    "tool_choice",
    "tools",
    "top_p",
    "user",
    "web_search_options",
];

#[derive(Debug, Error)]
pub(crate) enum CompletionsError {
    #[error("the provider answered with an error status")]
    Provider,
    #[error("the provider request could not be built: {0}")]
    Request(String),
    #[error("the provider request failed: {0}")]
    Transport(String),
    #[error("the provider answer could not be read: {0}")]
    Decode(String),
}

// B916's completion seam against the mock provider: the body names the mapped chat request, the headers
// carry the case's api key and extra headers, and a non-2xx answer is kept for B011's projection
pub(crate) struct HttpCompletions {
    client: ProxiedClient,
    deployment: ChatDeployment,
    failure: Mutex<Option<UpstreamResponse>>,
}

impl HttpCompletions {
    pub(crate) fn new(
        call: &Call,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> Result<Self, SdkHttpError> {
        let chat = OpenaiChat {
            model: call.model.clone(),
            messages: Vec::<ChatMessage>::new(),
            api_key: call.api_key.clone(),
            timeout_seconds: call.timeout_seconds,
        };
        let executor = native::executor(&chat, api_base, env)?;
        let Some(deployment) = executor.deployments.first().cloned() else {
            return Err(SdkHttpError::Invalid(format!(
                "no deployment serves {}",
                call.model
            )));
        };
        Ok(Self {
            client: executor.client,
            deployment,
            failure: Mutex::new(None),
        })
    }

    // The class, status and provider fields B011 derives from the recorded answer, with no Responses conversion
    pub(crate) fn project_failure(&self, model: &str) -> Result<Failure, SdkHttpError> {
        let recorded = self.failure.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(response) = recorded.as_ref() else {
            return Err(SdkHttpError::Invalid(
                "the provider answered an error the client did not record".to_owned(),
            ));
        };
        native::failure(model, response)
    }

    fn outbound_request(
        &self,
        arguments: &CompletionArguments,
    ) -> Result<(OutboundRequest, bool), CompletionsError> {
        let stream = streaming(arguments);
        let body = serde_json::to_vec(&provider_body(arguments))
            .map_err(|source| CompletionsError::Decode(source.to_string()))?;
        let request = OutboundRequest {
            method: Method::POST,
            url: self.deployment.endpoint.clone(),
            query: Vec::new(),
            headers: self.headers(arguments.fields.get("extra_headers"), stream)?,
            body: RequestBody::Json(body),
            deadline: self.deployment.timeout,
        };
        Ok((request, stream))
    }

    // A non-2xx answer is read whole and kept for B011's projection, then reported as the provider's failure
    async fn capture_failure(&self, response: StreamResponse) -> CompletionsError {
        match response.buffered(self.client.limits.body_bytes).await {
            Ok(upstream) => {
                let mut slot = self.failure.lock().unwrap_or_else(PoisonError::into_inner);
                *slot = Some(upstream);
                CompletionsError::Provider
            }
            Err(source) => CompletionsError::Transport(source.to_string()),
        }
    }

    fn headers(
        &self,
        extra: Option<&Value>,
        streaming: bool,
    ) -> Result<HeaderMap, CompletionsError> {
        let mut headers = HeaderMap::new();
        let credential = self
            .deployment
            .credential()
            .ok_or_else(|| CompletionsError::Request("the deployment has no api key".to_owned()))?;
        headers.insert(
            AUTHORIZATION,
            header(credential.expose_secret(), "authorization")?,
        );
        let accept = if streaming {
            "text/event-stream"
        } else {
            "application/json"
        };
        headers.insert(ACCEPT, HeaderValue::from_static(accept));
        if let Some(organization) = &self.deployment.organization {
            headers.insert(
                HeaderName::from_static("openai-organization"),
                header(organization, "openai-organization")?,
            );
        }
        if let Some(Value::Object(fields)) = extra {
            for (name, value) in fields {
                let Value::String(text) = value else {
                    return Err(CompletionsError::Request(format!(
                        "the extra header {name} is not text"
                    )));
                };
                let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
                    CompletionsError::Request(format!(
                        "the header name {name} is invalid: {source}"
                    ))
                })?;
                headers.insert(header_name, header(text, name)?);
            }
        }
        Ok(headers)
    }
}

fn header(text: &str, name: &str) -> Result<HeaderValue, CompletionsError> {
    HeaderValue::from_str(text).map_err(|source| {
        CompletionsError::Request(format!("the header {name} is invalid: {source}"))
    })
}

// The provider body is the mapped chat request, never the litellm arguments the handler also carries
fn provider_body(arguments: &CompletionArguments) -> Map<String, Value> {
    let mut body = Map::new();
    for name in BODY_PARAMS {
        if let Some(value) = arguments.fields.get(*name) {
            body.insert((*name).to_owned(), value.clone());
        }
    }
    body
}

fn streaming(arguments: &CompletionArguments) -> bool {
    matches!(arguments.fields.get("stream"), Some(Value::Bool(true)))
}

impl CompletionClient for HttpCompletions {
    type Error = CompletionsError;

    fn completion(
        &self,
        arguments: &CompletionArguments,
    ) -> impl Future<Output = Result<CompletionOutput, Self::Error>> + Send {
        async move {
            let (outbound, stream) = self.outbound_request(arguments)?;
            let response = self
                .client
                .send_following(outbound)
                .await
                .map_err(|source| CompletionsError::Transport(source.to_string()))?;
            if !response.response.status().is_success() {
                return Err(self.capture_failure(response).await);
            }
            if stream {
                return Ok(CompletionOutput::Stream(CompletionBody(
                    response.body_stream(),
                )));
            }
            let upstream = response
                .buffered(self.client.limits.body_bytes)
                .await
                .map_err(|source| CompletionsError::Transport(source.to_string()))?;
            let chat: ChatCompletionResponse = serde_json::from_slice(&upstream.body)
                .map_err(|source| CompletionsError::Decode(source.to_string()))?;
            Ok(CompletionOutput::Response(Box::new(chat)))
        }
    }
}
