use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::chat::response::ChatUsage;
use betterlite_openai::chat::{ChatMessage, MessageContent};
use http::header::{CONTENT_TYPE, HeaderValue};
use http::{HeaderMap, Method};
use indexmap::IndexMap;
use serde_json::Number;
use url::Url;

use crate::aiohttp_openai_dispatch::{AiohttpChatResponse, AiohttpChoice, AiohttpMessage};
use crate::operation::{
    ChatCompletion, ChatOperation, OperationError, OperationRequest, OperationResult, RequestKind,
    success,
};
use crate::prepared::Upstream;
use crate::prepared::body;
use crate::prepared::deployment::Deployment;

use super::Legacy;
use super::error;

// LiteLLM picks the CustomLLM method by these two flags alone
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomChatMethod {
    Completion,
    Streaming,
    Acompletion,
    Astreaming,
}

impl CustomChatMethod {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Completion => "completion",
            Self::Streaming => "streaming",
            Self::Acompletion => "acompletion",
            Self::Astreaming => "astreaming",
        }
    }
}

// custom_chat_llm_router: the async flag first, then a truthy stream, exactly as main.py chooses
#[must_use]
pub const fn custom_chat_llm_router(async_fn: bool, stream: Option<bool>) -> CustomChatMethod {
    match (async_fn, stream) {
        (true, Some(true)) => CustomChatMethod::Astreaming,
        (true, Some(false) | None) => CustomChatMethod::Acompletion,
        (false, Some(true)) => CustomChatMethod::Streaming,
        (false, Some(false) | None) => CustomChatMethod::Completion,
    }
}

// LiteLLM's _custom_api_first_output: resp.json()["data"][0]["output"][0]
pub fn decode_custom_output(body: &[u8]) -> Result<String, error::LegacyError> {
    let parsed: JsonValue =
        serde_json::from_slice(body).map_err(|source| error::LegacyError::Reply { source })?;
    let JsonValue::Object(fields) = parsed else {
        return Err(error::LegacyError::CustomOutput);
    };
    let Some(JsonValue::Array(data)) = fields.get("data") else {
        return Err(error::LegacyError::CustomOutput);
    };
    let Some(JsonValue::Object(first)) = data.first() else {
        return Err(error::LegacyError::CustomOutput);
    };
    let Some(JsonValue::Array(output)) = first.get("output") else {
        return Err(error::LegacyError::CustomOutput);
    };
    match output.first() {
        Some(JsonValue::Text(text)) => Ok(text.clone()),
        None
        | Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        ) => Err(error::LegacyError::CustomOutput),
    }
}

#[derive(Debug, Clone)]
pub(super) struct Call {
    pub(super) url: Url,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
    pub(super) timeout: Duration,
    pub(super) created: i64,
    pub(super) client_model: Option<String>,
}

fn unsupported(operation: &str) -> OperationError {
    OperationError::UnsupportedOperation {
        provider: String::from(Legacy::Custom.slug()),
        operation: String::from(operation),
    }
}

pub(super) fn prepare(
    request: &OperationRequest,
    deployment: &Deployment,
) -> Result<Call, OperationError> {
    match request.kind() {
        RequestKind::Chat(chat) => chat_call(chat, deployment),
        // _complete_custom answers a stream request with a buffered ModelResponse, which betterlite has no seam for
        RequestKind::Stream(_) => Err(unsupported("stream")),
        RequestKind::Embedding(_) | RequestKind::Other => Err(unsupported("provider endpoint")),
    }
}

fn chat_call(chat: &ChatOperation, deployment: &Deployment) -> Result<Call, OperationError> {
    let body::ChatRequestFields {
        params,
        .. // the prompt is rebuilt from the request messages below
    } = body::chat_request_fields(&chat.request)?;
    let inner = custom_params(chat, &params)?;
    let request_body = custom_body(&deployment.model, &params, inner)?;
    let url = Url::parse(&deployment.api_base)
        .map_err(|source| error::operation(error::LegacyError::Endpoint { source }))?;
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    Ok(Call {
        url,
        headers,
        body: serde_json::to_vec(&JsonValue::Object(request_body))
            .map_err(|source| OperationError::Encode { source })?,
        timeout: deployment.timeout,
        created: i64::try_from(seconds).unwrap_or(0),
        client_model: chat.request.model.clone(),
    })
}

// _complete_custom sends the joined prompt beside every call parameter in one params object
fn custom_params(
    chat: &ChatOperation,
    params: &IndexMap<String, JsonValue>,
) -> Result<IndexMap<String, JsonValue>, OperationError> {
    let prompt = prompt_text(&chat.request.messages)?;
    let mut inner = IndexMap::new();
    inner.insert(
        String::from("prompt"),
        JsonValue::Array(vec![JsonValue::Text(prompt)]),
    );
    inner.insert(
        String::from("max_tokens"),
        number_or_null(chat.request.max_tokens.map(Number::from).as_ref()),
    );
    inner.insert(
        String::from("temperature"),
        number_or_null(chat.request.temperature.as_ref()),
    );
    inner.insert(
        String::from("top_p"),
        number_or_null(chat.request.top_p.as_ref()),
    );
    inner.insert(
        String::from("top_k"),
        params.get("top_k").cloned().unwrap_or(JsonValue::Null),
    );
    Ok(inner)
}

// a number field goes out as the number when set and as an explicit null otherwise
fn number_or_null(number: Option<&Number>) -> JsonValue {
    number.cloned().map_or(JsonValue::Null, JsonValue::Number)
}

// extra_body expands last, so it may replace the model or the whole params object
fn custom_body(
    model: &str,
    params: &IndexMap<String, JsonValue>,
    inner: IndexMap<String, JsonValue>,
) -> Result<IndexMap<String, JsonValue>, OperationError> {
    let mut request_body = IndexMap::new();
    request_body.insert(String::from("model"), JsonValue::Text(String::from(model)));
    request_body.insert(String::from("params"), JsonValue::Object(inner));
    match params.get("extra_body") {
        None | Some(JsonValue::Null) => {}
        Some(JsonValue::Object(extra)) => {
            for (name, value) in extra {
                request_body.insert(name.clone(), value.clone());
            }
        }
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) | JsonValue::Array(_),
        ) => return Err(error::operation(error::LegacyError::ExtraBodyNotObject)),
    }
    Ok(request_body)
}

// _complete_custom joins message["content"] with one space and sends nothing else
fn prompt_text(messages: &[ChatMessage]) -> Result<String, OperationError> {
    let mut parts = Vec::with_capacity(messages.len());
    for (index, message) in messages.iter().enumerate() {
        let text = match &message.content {
            Some(MessageContent::Text(text)) => text,
            None | Some(MessageContent::Parts(_)) => {
                return Err(error::operation(error::LegacyError::Content { index }));
            }
        };
        parts.push(text.as_str());
    }
    Ok(parts.join(" "))
}

pub(super) async fn execute(
    client: &ProxiedClient,
    call: &Call,
) -> Result<Upstream, OperationError> {
    let outbound = OutboundRequest {
        method: Method::POST,
        url: call.url.clone(),
        query: Vec::new(),
        headers: call.headers.clone(),
        body: RequestBody::Json(call.body.clone()),
        deadline: call.timeout,
    };
    // litellm.module_level_client follows no redirect, so the deprecated path does not either
    let response = client.send(outbound).await.map_err(OperationError::from)?;
    let buffered = response
        .buffered(client.limits.body_bytes)
        .await
        .map_err(OperationError::from)?;
    Ok(Upstream::Buffered(buffered))
}

pub(super) fn decode(call: &Call, upstream: Upstream) -> Result<OperationResult, OperationError> {
    match upstream {
        Upstream::Buffered(buffered) => {
            let checked = success(buffered)?;
            let content = decode_custom_output(&checked.body).map_err(error::operation)?;
            let response = AiohttpChatResponse {
                id: None,
                object: Some(String::from("chat.completion")),
                created: u64::try_from(call.created).ok(),
                model: call.client_model.clone().filter(|model| !model.is_empty()),
                choices: vec![AiohttpChoice {
                    index: 0,
                    message: AiohttpMessage {
                        role: Some(String::from("assistant")),
                        content: Some(content),
                        function_call: None,
                        tool_calls: None,
                        extra: IndexMap::new(),
                    },
                    // Choices.__init__ fills a missing finish_reason with "stop", and the reply's choice is the default one
                    finish_reason: Some(String::from("stop")),
                    logprobs: None,
                    provider_specific_fields: None,
                    extra: IndexMap::new(),
                }],
                system_fingerprint: None,
                // LiteLLM leaves the zero usage it starts with; nothing is counted for this path
                usage: ChatUsage::default(),
            };
            Ok(OperationResult::Chat(ChatCompletion::AiohttpOpenAi(
                Box::new(response),
            )))
        }
        Upstream::Streaming(_) => Err(unsupported("stream")),
        Upstream::Anthropic(_) => Err(unsupported("provider endpoint")),
    }
}
