use std::time::Duration;

use betterlite_prompts::arguments::json::Json;
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap, Method};
use url::Url;

use crate::operation::OperationError;
use crate::prepared::{PreparedChat, PreparedHttp, PreparedRequest};
use crate::transform_retry::TransformRetry;

use super::call::{ChatCall, Composition, EmbeddingCall, TextCall};
use super::error::AzureProviderError;
use super::resolve::{merge_defaults, resolve_chat, resolve_embedding, resolve_text};
use super::wire::{self, Wire};

const DEFAULT_TIMEOUT_SECONDS: f64 = 6000.0;

fn sensitive(name: &str) -> bool {
    name.eq_ignore_ascii_case("authorization")
        || name.eq_ignore_ascii_case("api-key")
        || name.eq_ignore_ascii_case("proxy-authorization")
}

fn rejected(message: String) -> AzureProviderError {
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
        let mut value =
            HeaderValue::try_from(value.as_str()).map_err(|source| AzureProviderError::Header {
                name: name.clone(),
                source,
            })?;
        if sensitive(name) {
            value.set_sensitive(true);
        }
        headers.insert(header, value);
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

pub fn chat_request(http: PreparedHttp, stream: bool) -> PreparedRequest {
    let prepared = PreparedChat {
        http,
        retry: TransformRetry::DISABLED,
        client_model: None,
    };
    if stream {
        PreparedRequest::Stream(Box::new(prepared))
    } else {
        PreparedRequest::Chat(Box::new(prepared))
    }
}

// The bool is json_mode, which the handler pops and the reply conversion reads.
pub fn prepare_chat_http(
    composition: &Composition,
    call: ChatCall,
) -> Result<(PreparedHttp, bool), AzureProviderError> {
    let base_model = call.credentials.base_model.clone();
    let detection = base_model.clone().unwrap_or_else(|| call.model.clone());
    let timeout = call.credentials.timeout;
    let stream = call.stream;
    let model = call.model;
    let messages = call.messages;
    let mut optional = call.optional;
    // the check runs on the caller's own version, before the environment and default fallback
    if call
        .supplied
        .as_ref()
        .is_some_and(|client| client.discarded(call.credentials.api_version.as_deref()))
    {
        tracing::debug!(
            "the supplied Azure client's api_version changed, so the client is rebuilt"
        );
    }
    let resolved = resolve_chat(composition, call.credentials, &detection)?;
    if resolved.o_series {
        merge_defaults(&mut optional, &composition.o_series_defaults)?;
    } else {
        merge_defaults(&mut optional, &composition.chat_defaults)?;
    }
    let wire = wire::chat(
        &resolved,
        &model,
        &messages,
        optional,
        stream,
        base_model.as_deref(),
        composition.openai_system_messages_first,
    )?;
    let json_mode = wire.json_mode;
    Ok((prepared_http(wire, timeout)?, json_mode))
}

pub fn prepare_chat(
    composition: &Composition,
    call: ChatCall,
) -> Result<PreparedRequest, OperationError> {
    let stream = call.stream;
    let (http, _json_mode) = prepare_chat_http(composition, call)?;
    Ok(chat_request(http, stream))
}

pub fn prepare_embedding_http(
    composition: &Composition,
    call: EmbeddingCall,
) -> Result<PreparedHttp, AzureProviderError> {
    let timeout = call.credentials.timeout;
    let model = call.model;
    let request = call.request;
    let resolved = resolve_embedding(composition, call.credentials)?;
    let wire = wire::embedding(&resolved, &model, &request)?;
    prepared_http(wire, timeout)
}

pub fn prepare_embedding(
    composition: &Composition,
    call: EmbeddingCall,
) -> Result<PreparedRequest, OperationError> {
    let http = prepare_embedding_http(composition, call)?;
    Ok(PreparedRequest::Embedding(Box::new(PreparedChat {
        http,
        retry: TransformRetry::DISABLED,
        client_model: None,
    })))
}

// The bool is the text post_call condition: a truthy stream or acompletion.
pub fn prepare_text_http(
    composition: &Composition,
    call: TextCall,
) -> Result<(PreparedHttp, bool), AzureProviderError> {
    let timeout = call.credentials.timeout;
    let acompletion = call.acompletion;
    let model = call.model;
    let messages = call.messages;
    let mut optional = call.optional;
    let post_call = acompletion || optional.get("stream").is_some_and(Json::is_truthy);
    let resolved = resolve_text(composition, call.credentials)?;
    merge_defaults(&mut optional, &composition.chat_defaults)?;
    let wire = wire::text(&resolved, &model, &messages, optional)?;
    Ok((prepared_http(wire, timeout)?, post_call))
}
