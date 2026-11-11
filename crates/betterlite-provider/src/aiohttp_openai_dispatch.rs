use std::collections::BTreeMap;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::chat::{FunctionCall, ToolCall};
use betterlite_openai::openai_base_config::CatalogCapabilities;
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue, Method};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use url::Url;

use crate::openai_dispatch::chat_body;
use crate::operation::{
    ChatCompletion, ChatOperation, OperationError, OperationRequest, OperationResult, success,
};
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedChat, PreparedHttp, PreparedRequest, Upstream};
use crate::transform_retry::{TransformOnUnprocessable, TransformRetry};

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
}

#[skip_serializing_none]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct AiohttpChoice {
    #[serde(default)]
    pub index: u32,
    pub message: AiohttpMessage,
    #[serde(default = "default_finish_reason")]
    pub finish_reason: String,
    pub logprobs: Option<JsonValue>,
    pub provider_specific_fields: Option<BTreeMap<String, JsonValue>>,
    #[serde(flatten)]
    pub extra: IndexMap<String, JsonValue>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct AiohttpMessage {
    pub role: String,
    #[serialize_always]
    pub content: Option<String>,
    pub function_call: Option<FunctionCall>,
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(flatten)]
    pub extra: IndexMap<String, JsonValue>,
}

fn default_finish_reason() -> String {
    String::from("stop")
}

impl AiohttpOpenAi {
    pub fn prepare(
        &self,
        request: &OperationRequest,
        deployment: &Deployment,
        catalog: &CatalogCapabilities,
    ) -> Result<PreparedRequest, OperationError> {
        if let OperationRequest::Chat(chat) = request {
            return chat_request(chat, deployment, catalog, false);
        }
        if let OperationRequest::Stream(chat) = request {
            return chat_request(chat, deployment, catalog, true);
        }
        Err(unsupported())
    }

    pub fn decode(
        &self,
        prepared: &PreparedRequest,
        upstream: Upstream,
    ) -> Result<OperationResult, OperationError> {
        match upstream {
            Upstream::Streaming(response) => {
                if let PreparedRequest::Stream(_) = prepared {
                    return Ok(OperationResult::Stream(Box::new(response)));
                }
                Err(unsupported())
            }
            Upstream::Buffered(response) => {
                if let PreparedRequest::Chat(_) = prepared {
                    let checked = success(response)?;
                    let mut decoded: AiohttpChatResponse = serde_json::from_slice(&checked.body)
                        .map_err(|source| OperationError::Decode {
                            operation: "chat",
                            source,
                        })?;
                    normalize(&mut decoded);
                    return Ok(OperationResult::Chat(ChatCompletion::AiohttpOpenAi(
                        Box::new(decoded),
                    )));
                }
                Err(unsupported())
            }
        }
    }
}

impl TransformOnUnprocessable for AiohttpOpenAi {}

#[must_use]
fn unsupported() -> OperationError {
    OperationError::UnsupportedOperation {
        provider: String::from("aiohttp_openai"),
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
    let body = chat_body(&chat.request, &chat.model, catalog)?;
    let written = complete_url(&deployment.api_base);
    let url = Url::parse(&written).map_err(|source| OperationError::Endpoint {
        value: written,
        source,
    })?;
    // the aiohttp transformation answers Bearer {api_key} and never reads an environment key
    let bearer = deployment.api_key.as_ref().map_or_else(
        || String::from("Bearer None"),
        |key| format!("Bearer {}", key.expose_secret()),
    );
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::try_from(bearer).map_err(|source| OperationError::InvalidHeader {
            name: String::from("authorization"),
            source,
        })?,
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let http = PreparedHttp {
        method: Method::POST,
        url,
        headers,
        body,
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

// LiteLLM passes the raw choice to its typed records, whose proxy serialization drops every null
fn normalize(response: &mut AiohttpChatResponse) {
    for choice in &mut response.choices {
        choice
            .extra
            .retain(|_name, value| !matches!(value, JsonValue::Null));
        choice
            .message
            .extra
            .retain(|_name, value| !matches!(value, JsonValue::Null));
    }
}

mod tests;
