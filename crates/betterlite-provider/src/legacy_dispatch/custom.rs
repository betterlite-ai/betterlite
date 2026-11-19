use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::chat::response::ChatUsage;
use betterlite_openai::chat::{ChatMessage, MessageContent};
use betterlite_openai::openai_base_config::Object;
use http::header::{CONTENT_TYPE, HeaderValue};
use http::{HeaderMap, Method};
use indexmap::IndexMap;
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

pub(super) fn prepare(
    request: &OperationRequest,
    deployment: &Deployment,
) -> Result<Call, OperationError> {
    match request.kind() {
        RequestKind::Chat(chat) => chat_call(chat, deployment),
        // _complete_custom answers a stream request with a buffered ModelResponse, which betterlite has no seam for
        RequestKind::Stream(_) => Err(OperationError::UnsupportedOperation {
            provider: String::from(Legacy::Custom.slug()),
            operation: String::from("stream"),
        }),
        RequestKind::Embedding(_) | RequestKind::Other => {
            Err(OperationError::UnsupportedOperation {
                provider: String::from(Legacy::Custom.slug()),
                operation: String::from("provider endpoint"),
            })
        }
    }
}

fn chat_call(chat: &ChatOperation, deployment: &Deployment) -> Result<Call, OperationError> {
    let body::ChatRequestFields {
        messages: _,
        params,
    } = body::chat_request_fields(&chat.request)?;
    let prompt = prompt_text(&chat.request.messages)?;
    let mut inner = Object::new();
    inner.insert(
        String::from("prompt"),
        serde_json::Value::Array(vec![serde_json::Value::String(prompt)]),
    );
    inner.insert(
        String::from("max_tokens"),
        match chat.request.max_tokens {
            Some(limit) => serde_json::Value::from(limit),
            None => serde_json::Value::Null,
        },
    );
    inner.insert(
        String::from("temperature"),
        match chat.request.temperature.as_ref() {
            Some(number) => serde_json::Value::Number(number.clone()),
            None => serde_json::Value::Null,
        },
    );
    inner.insert(
        String::from("top_p"),
        match chat.request.top_p.as_ref() {
            Some(number) => serde_json::Value::Number(number.clone()),
            None => serde_json::Value::Null,
        },
    );
    inner.insert(
        String::from("top_k"),
        match params.get("top_k") {
            Some(value) => {
                serde_json::to_value(value).map_err(|source| OperationError::Encode { source })?
            }
            None => serde_json::Value::Null,
        },
    );
    let mut request_body = Object::new();
    request_body.insert(
        String::from("model"),
        serde_json::Value::String(deployment.model.clone()),
    );
    request_body.insert(String::from("params"), serde_json::Value::Object(inner));
    // extra_body expands last, so it may replace the model or the whole params object
    match params.get("extra_body") {
        None | Some(JsonValue::Null) => {}
        Some(JsonValue::Object(extra)) => {
            for (name, value) in extra {
                let converted = serde_json::to_value(value)
                    .map_err(|source| OperationError::Encode { source })?;
                request_body.insert(name.clone(), converted);
            }
        }
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) | JsonValue::Array(_),
        ) => return Err(error::operation(error::LegacyError::ExtraBodyNotObject)),
    }
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
        body: serde_json::to_vec(&serde_json::Value::Object(request_body))
            .map_err(|source| OperationError::Encode { source })?,
        timeout: deployment.timeout,
        created: i64::try_from(seconds).unwrap_or(0),
        client_model: chat.request.model.clone(),
    })
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
    let Upstream::Buffered(response) = upstream else {
        return Err(OperationError::UnsupportedOperation {
            provider: String::from(Legacy::Custom.slug()),
            operation: String::from("stream"),
        });
    };
    let checked = success(response)?;
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
