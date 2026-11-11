use std::collections::BTreeMap;

use betterlite_http::proxy::ProxiedClient;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::chat::response::ChatUsage;
use betterlite_openai::chat::{FunctionCall, ToolCall, choice_finish_reason};
use betterlite_openai::openai_base_config::CatalogCapabilities;
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue, Method};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::operation::{
    ChatCompletion, ChatOperation, OperationError, OperationRequest, OperationResult, RequestKind,
    success,
};
use crate::prepared::body::{authorization_value, chat_fields, encode};
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedChat, PreparedHttp, PreparedKind, PreparedRequest, Upstream};
use crate::provider::Adapter;
use crate::transform_retry::{TransformOnUnprocessable, TransformRetry, send_with_retry};

// LiteLLM's aiohttp_openai transformation defaults the base to the bare OpenAI host, without its /v1 segment
pub const DEFAULT_AIOHTTP_BASE: &str = "https://api.openai.com";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AiohttpOpenAi;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AiohttpOpenAiRequest;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AiohttpOpenAiResult;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AiohttpOpenAiPrepared;

#[skip_serializing_none]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct AiohttpChatResponse {
    pub id: Option<String>,
    pub object: Option<String>,
    pub created: Option<u64>,
    pub model: Option<String>,
    pub choices: Vec<AiohttpChoice>,
    pub system_fingerprint: Option<String>,
    // LiteLLM copies no usage from the upstream, so the zero one it starts with stays
    #[serde(default, skip_deserializing)]
    pub usage: ChatUsage,
}

// finish_reason and role arrive optional and leave filled in, the way LiteLLM's Choices and Message build them
#[skip_serializing_none]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct AiohttpChoice {
    #[serde(default)]
    pub index: u32,
    pub message: AiohttpMessage,
    pub finish_reason: Option<String>,
    pub logprobs: Option<JsonValue>,
    pub provider_specific_fields: Option<BTreeMap<String, JsonValue>>,
    #[serde(flatten)]
    pub extra: IndexMap<String, JsonValue>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct AiohttpMessage {
    pub role: Option<String>,
    #[serialize_always]
    pub content: Option<String>,
    pub function_call: Option<FunctionCall>,
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(flatten)]
    pub extra: IndexMap<String, JsonValue>,
}

impl Adapter for AiohttpOpenAi {
    const NAME: &'static str = "aiohttp_openai";

    fn prepare(
        &self,
        request: &OperationRequest,
        deployment: &Deployment,
        catalog: &CatalogCapabilities,
    ) -> Result<PreparedRequest, OperationError> {
        match request.kind() {
            RequestKind::Chat(chat) => chat_request(chat, deployment, catalog, false),
            RequestKind::Stream(chat) => chat_request(chat, deployment, catalog, true),
            RequestKind::Embedding(_) | RequestKind::Other => Err(unsupported()),
        }
    }

    async fn execute(
        &self,
        client: &ProxiedClient,
        prepared: &PreparedRequest,
        deployment: &Deployment,
    ) -> Result<Upstream, OperationError> {
        send_with_retry(client, prepared, deployment, self).await
    }

    fn decode(
        &self,
        prepared: &PreparedRequest,
        upstream: Upstream,
    ) -> Result<OperationResult, OperationError> {
        match (prepared.kind(), upstream) {
            (PreparedKind::Chat(chat), Upstream::Buffered(response)) => {
                let checked = success(response)?;
                let mut decoded: AiohttpChatResponse = serde_json::from_slice(&checked.body)
                    .map_err(|source| OperationError::Decode {
                        operation: "chat",
                        source,
                    })?;
                normalize(&mut decoded);
                // the proxy answers with the model the client asked for, not the one the upstream reported
                if let Some(model) = chat.client_model.as_ref().filter(|model| !model.is_empty()) {
                    decoded.model = Some(model.clone());
                }
                Ok(OperationResult::Chat(ChatCompletion::AiohttpOpenAi(
                    Box::new(decoded),
                )))
            }
            (PreparedKind::Stream(_), Upstream::Streaming(response)) => {
                Ok(OperationResult::Stream(Box::new(response)))
            }
            // a refused stream arrives as a finished body, which the caller renders as the provider's own error
            (PreparedKind::Stream(_), Upstream::Buffered(response)) => {
                success(response)?;
                Err(unsupported())
            }
            (PreparedKind::Chat(_) | PreparedKind::Embedding(_), Upstream::Streaming(_))
            | (PreparedKind::Embedding(_), Upstream::Buffered(_))
            | (PreparedKind::Other, Upstream::Buffered(_) | Upstream::Streaming(_)) => {
                Err(unsupported())
            }
        }
    }
}

impl TransformOnUnprocessable for AiohttpOpenAi {}

fn unsupported() -> OperationError {
    OperationError::UnsupportedOperation {
        provider: String::from(AiohttpOpenAi::NAME),
        operation: String::from("provider endpoint"),
    }
}

#[must_use]
pub fn complete_url(api_base: &str) -> String {
    if api_base.ends_with("/chat/completions") {
        api_base.to_owned()
    } else {
        format!("{api_base}/chat/completions")
    }
}

fn chat_request(
    chat: &ChatOperation,
    deployment: &Deployment,
    catalog: &CatalogCapabilities,
    stream: bool,
) -> Result<PreparedRequest, OperationError> {
    let mut fields = chat_fields(&chat.request, &chat.model, catalog)?;
    // OpenAILikeChatConfig.map_openai_params sends max_completion_tokens as max_tokens
    if let Some(limit) = fields.shift_remove("max_completion_tokens") {
        fields.insert(String::from("max_tokens"), limit);
    }
    // the router sets stream=False on a buffered call and this transformation sends every optional param
    if !stream {
        fields
            .entry(String::from("stream"))
            .or_insert(JsonValue::Bool(false));
    }
    // the aiohttp transformation answers Bearer {api_key} and never reads an environment key
    let bearer = deployment.api_key.as_ref().map_or_else(
        || String::from("Bearer None"),
        |key| format!("Bearer {}", key.expose_secret()),
    );
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, authorization_value(&bearer)?);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let http = PreparedHttp {
        method: Method::POST,
        url: deployment.endpoint.clone(),
        headers,
        body: encode(fields)?,
        deadline: deployment.timeout,
        body_limit: None,
    };
    let prepared = PreparedChat {
        http,
        retry: TransformRetry::DISABLED,
        client_model: chat.request.model.clone(),
    };
    Ok(if stream {
        PreparedRequest::Stream(Box::new(prepared))
    } else {
        PreparedRequest::Chat(Box::new(prepared))
    })
}

// LiteLLM builds Choices(**choice): it maps the finish reason, fills the role and drops nulls and an empty tool_calls
fn normalize(response: &mut AiohttpChatResponse) {
    for choice in &mut response.choices {
        let raw = choice.finish_reason.take();
        let (reason, native) = choice_finish_reason(raw.as_deref());
        choice.finish_reason = Some(reason);
        if let Some(given) = raw
            && !native.is_empty()
        {
            choice
                .provider_specific_fields
                .get_or_insert_default()
                .insert(String::from("native_finish_reason"), JsonValue::Text(given));
        }
        choice
            .extra
            .retain(|_name, value| !matches!(value, JsonValue::Null));
        let message = &mut choice.message;
        message
            .role
            .get_or_insert_with(|| String::from("assistant"));
        if message.tool_calls.as_ref().is_some_and(Vec::is_empty) {
            message.tool_calls = None;
        }
        message
            .extra
            .retain(|_name, value| !matches!(value, JsonValue::Null));
    }
}

mod tests;
