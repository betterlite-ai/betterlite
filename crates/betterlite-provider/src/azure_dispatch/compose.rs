use super::call::{ChatCall, Composition, EmbeddingCall, Operation, Request, TextCall};
use super::error::AzureProviderError;
use super::resolve::{merge_defaults, resolve};
use super::wire::{self, Wire};
use crate::prepared::{PreparedChat, PreparedHttp, PreparedRequest};
use crate::transform_retry::TransformRetry;
use betterlite_prompts::arguments::json::Json;
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap, Method};
use std::time::Duration;
use url::Url;
const DEFAULT_TIMEOUT_SECONDS: f64 = 6000.0;
pub enum ReplyMode {
    Chat { json_mode: bool },
    Text { post_call: bool },
    Embedding,
}
pub struct Prepared {
    pub http: PreparedHttp,
    pub mode: ReplyMode,
    // the model LiteLLM detects branches with: base_model or model, on the chat path only
    pub detection: Option<String>,
    pub stream: bool,
}
const fn rejected(message: String) -> AzureProviderError {
    AzureProviderError::Rejected {
        status: 400,
        message,
    }
}
fn prepared_http(wire: Wire, timeout: Option<f64>) -> Result<PreparedHttp, AzureProviderError> {
    let url = Url::parse(&wire.url).map_err(|source| rejected(source.to_string()))?;
    let mut headers = HeaderMap::new();
    for (name, value) in &wire.headers {
        let header =
            HeaderName::try_from(name.as_str()).map_err(|source| rejected(source.to_string()))?;
        let mut header_value =
            HeaderValue::try_from(value.as_str()).map_err(|source| AzureProviderError::Header {
                name: name.clone(),
                source,
            })?;
        if ["authorization", "api-key", "proxy-authorization"]
            .iter()
            .any(|secret| name.eq_ignore_ascii_case(secret))
        {
            header_value.set_sensitive(true);
        }
        headers.insert(header, header_value);
    }
    let body = serde_json::to_vec(&Json::Object(wire.body))
        .map_err(|source| AzureProviderError::Encode { source })?;
    let deadline = Duration::try_from_secs_f64(timeout.unwrap_or(DEFAULT_TIMEOUT_SECONDS))
        .map_err(|source| AzureProviderError::Rejected {
            status: 500,
            message: source.to_string(),
        })?;
    Ok(PreparedHttp {
        method: Method::POST,
        url,
        headers,
        body,
        deadline,
        body_limit: None,
    })
}
// the supplied client changes only this debug log, so its mutants are equivalent
#[cfg_attr(test, mutants::skip)]
fn check_supplied(call: &ChatCall) {
    if call
        .supplied
        .as_ref()
        .is_some_and(|client| client.discarded(call.credentials.api_version.as_deref()))
    {
        tracing::debug!(
            "the supplied Azure client's api_version changed, so the client is rebuilt"
        );
    }
}
fn chat_part(composition: &Composition, call: &ChatCall) -> Result<Prepared, AzureProviderError> {
    let detection = call
        .credentials
        .base_model
        .clone()
        .unwrap_or_else(|| call.model.clone());
    check_supplied(call);
    let resolved = resolve(
        composition,
        call.credentials.clone(),
        Operation::Chat,
        Some(detection.as_str()),
    )?;
    let mut optional = call.optional.clone();
    if resolved.o_series {
        merge_defaults(&mut optional, &composition.o_series_defaults)?;
    } else {
        merge_defaults(&mut optional, &composition.chat_defaults)?;
    }
    let wire = wire::chat(
        &resolved,
        call,
        optional,
        composition.openai_system_messages_first,
    )?;
    let json_mode = wire.json_mode;
    let http = prepared_http(wire, call.credentials.timeout)?;
    Ok(Prepared {
        http,
        mode: ReplyMode::Chat { json_mode },
        detection: Some(detection),
        stream: call.stream,
    })
}
fn text_part(composition: &Composition, call: &TextCall) -> Result<Prepared, AzureProviderError> {
    let mut optional = call.optional.clone();
    let post_call = call.acompletion || optional.get("stream").is_some_and(Json::is_truthy);
    let resolved = resolve(composition, call.credentials.clone(), Operation::Text, None)?;
    merge_defaults(&mut optional, &composition.chat_defaults)?;
    let wire = wire::text(&resolved, call, optional)?;
    let http = prepared_http(wire, call.credentials.timeout)?;
    Ok(Prepared {
        http,
        mode: ReplyMode::Text { post_call },
        detection: None,
        stream: false,
    })
}
fn embedding_part(
    composition: &Composition,
    call: &EmbeddingCall,
) -> Result<Prepared, AzureProviderError> {
    let resolved = resolve(
        composition,
        call.credentials.clone(),
        Operation::Embedding,
        None,
    )?;
    let wire = wire::embedding(&resolved, &call.model, &call.request)?;
    let http = prepared_http(wire, call.credentials.timeout)?;
    Ok(Prepared {
        http,
        mode: ReplyMode::Embedding,
        detection: None,
        stream: false,
    })
}
pub fn prepare(
    composition: &Composition,
    request: Request,
) -> Result<Prepared, AzureProviderError> {
    match request {
        Request::Chat(call) => chat_part(composition, &call),
        Request::Text(call) => text_part(composition, &call),
        Request::Embedding(call) => embedding_part(composition, &call),
    }
}
// The request the adapter sends; text travels here as a chat variant for its own reply projection.
#[must_use]
pub fn prepared_request(prepared: Prepared) -> PreparedRequest {
    let chat = PreparedChat {
        http: prepared.http,
        retry: TransformRetry::DISABLED,
        client_model: prepared.detection,
    };
    if matches!(prepared.mode, ReplyMode::Embedding) {
        return PreparedRequest::Embedding(Box::new(chat));
    }
    if prepared.stream {
        PreparedRequest::Stream(Box::new(chat))
    } else {
        PreparedRequest::Chat(Box::new(chat))
    }
}
