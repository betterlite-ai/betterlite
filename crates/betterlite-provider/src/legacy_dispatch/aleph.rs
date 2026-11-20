use std::collections::BTreeMap;
use std::io::Error as IoError;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::sections::proxy_custom_dispatch::Settings;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::aleph_alpha::config::AlephAlphaConfig;
use betterlite_openai::aleph_alpha::reply::{decode_aleph_reply, resolve_usage};
use betterlite_openai::aleph_alpha::request::{
    CompletionInput, CompletionRequest, completion_request,
};
use betterlite_openai::openai_base_config::{CatalogCapabilities, Object};
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};
use http::Method;
use indexmap::IndexMap;
use serde_json::Error as JsonError;

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

// LiteLLM's DEFAULT_MAX_TOKENS, which AlephAlphaConfig defaults maximum_tokens to
const ALEPH_MAX_TOKENS: i64 = 4096;

#[derive(Debug, Clone)]
pub(super) struct Call {
    pub(super) request: CompletionRequest,
    pub(super) created: i64,
    pub(super) client_model: Option<String>,
    timeout: Duration,
}

pub(super) fn prepare(
    request: &OperationRequest,
    deployment: &Deployment,
) -> Result<Call, OperationError> {
    match request.kind() {
        RequestKind::Chat(chat) => chat_call(chat, deployment),
        // LiteLLM wraps the deprecated handler's answer in CustomStreamWrapper, which betterlite has no seam for
        RequestKind::Stream(_) => Err(OperationError::UnsupportedOperation {
            provider: String::from(Legacy::AlephAlpha.slug()),
            operation: String::from("stream"),
        }),
        RequestKind::Embedding(_) | RequestKind::Other => {
            Err(OperationError::UnsupportedOperation {
                provider: String::from(Legacy::AlephAlpha.slug()),
                operation: String::from("provider endpoint"),
            })
        }
    }
}

fn chat_call(chat: &ChatOperation, deployment: &Deployment) -> Result<Call, OperationError> {
    let body::ChatRequestFields {
        messages: _ignored,
        params,
    } = body::chat_request_fields(&chat.request)?;
    let optional_params = optional_params(params)?;
    let config = AlephAlphaConfig::new(ALEPH_MAX_TOKENS, Object::new()).map_err(|error| {
        error::operation(error::LegacyError::Settings {
            message: error.to_string(),
        })
    })?;
    let settings = Settings::default();
    let api_key = settings.resolve_aleph_alpha_key(deployment.api_key.as_ref(), None);
    let api_base = settings.resolve_aleph_alpha_base(Some(deployment.api_base.as_str()), None);
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let created = i64::try_from(seconds).unwrap_or(0);
    let input = CompletionInput {
        model: &deployment.model,
        messages: &chat.request.messages,
        api_base: &api_base,
        api_key: api_key.as_ref(),
        optional_params: &optional_params,
        config: &config,
        created,
    };
    Ok(Call {
        request: completion_request(&input).map_err(error::aleph)?,
        created,
        client_model: chat.request.model.clone(),
        timeout: deployment.timeout,
    })
}

// Aleph Alpha is not OpenAI-compatible: every non-null call param is forwarded, nulls are not
fn optional_params(params: IndexMap<String, JsonValue>) -> Result<Object, OperationError> {
    let mut optional = Object::new();
    for (name, value) in params {
        if matches!(value, JsonValue::Null) {
            continue;
        }
        let converted =
            serde_json::to_value(value).map_err(|source| OperationError::Encode { source })?;
        let wire_name = if name == "max_completion_tokens" {
            String::from("max_tokens")
        } else {
            name
        };
        optional.insert(wire_name, converted);
    }
    Ok(optional)
}

pub(super) async fn execute(
    client: &ProxiedClient,
    call: &Call,
) -> Result<Upstream, OperationError> {
    let body = serde_json::to_vec(&call.request.body)
        .map_err(|source| OperationError::Encode { source })?;
    let outbound = OutboundRequest {
        method: Method::POST,
        url: call.request.url.clone(),
        query: Vec::new(),
        headers: call.request.headers.clone(),
        body: RequestBody::Json(body),
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
            provider: String::from(Legacy::AlephAlpha.slug()),
            operation: String::from("stream"),
        });
    };
    let checked = success(response)?;
    let choices = decode_aleph_reply(checked.status, &checked.body).map_err(error::aleph)?;
    let data = assets::bundled(Encoding::Cl100kBase).ok_or_else(|| OperationError::Decode {
        operation: "chat",
        source: JsonError::io(IoError::other("cl100k_base is not bundled")),
    })?;
    let engine = TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &data).map_err(|source| {
        OperationError::Decode {
            operation: "chat",
            source: JsonError::io(IoError::other(source.to_string())),
        }
    })?;
    let tokenizer = Engine::Tiktoken(Arc::new(engine));
    let usage = resolve_usage(&call.request.prompt, &choices, &tokenizer).map_err(error::aleph)?;
    let mut output = Vec::with_capacity(choices.len());
    for choice in choices {
        let native = if choice.provider_specific_fields.is_empty() {
            None
        } else {
            let serialized = serde_json::to_value(&choice.provider_specific_fields)
                .map_err(|source| OperationError::Encode { source })?;
            let converted: BTreeMap<String, JsonValue> = serde_json::from_value(serialized)
                .map_err(|source| OperationError::Decode {
                    operation: "chat",
                    source,
                })?;
            Some(converted)
        };
        output.push(AiohttpChoice {
            index: choice.index,
            message: AiohttpMessage {
                role: Some(choice.message.role),
                content: choice.message.content,
                function_call: None,
                tool_calls: choice.message.tool_calls,
                extra: IndexMap::new(),
            },
            finish_reason: choice.finish_reason,
            logprobs: None,
            provider_specific_fields: native,
            extra: IndexMap::new(),
        });
    }
    let response = AiohttpChatResponse {
        id: None,
        object: Some(String::from("chat.completion")),
        created: u64::try_from(call.created).ok(),
        model: call.client_model.clone().filter(|model| !model.is_empty()),
        choices: output,
        system_fingerprint: None,
        usage,
    };
    Ok(OperationResult::Chat(ChatCompletion::AiohttpOpenAi(
        Box::new(response),
    )))
}
