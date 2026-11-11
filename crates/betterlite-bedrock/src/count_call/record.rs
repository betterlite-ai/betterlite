use std::str::FromStr as _;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json;
use betterlite_openai::error::{ErrorRequest, ErrorResponse};
use http::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use serde::de::DeserializeOwned;

use crate::bedrock_settings::{
    BedrockError, ErrorHeaders, ErrorInput, ErrorReply, HeaderError, ResponseBody,
};
use crate::count_tokens::response::transform_response;
use crate::count_tokens::transform_request;

use super::error::CountCallError;
use super::handler::{CountTokensHandlerRequest, ParamMap};
use super::reply::{CountResponse, decode_bedrock_count_reply};

pub(super) enum HandlerFailure {
    Bedrock(Box<BedrockError>),
    Processing(Box<CountCallError>),
}

impl From<CountCallError> for HandlerFailure {
    fn from(source: CountCallError) -> Self {
        Self::Processing(Box::new(source))
    }
}

impl From<HeaderError> for HandlerFailure {
    fn from(source: HeaderError) -> Self {
        Self::Processing(Box::new(CountCallError::ErrorHeader { source }))
    }
}

pub(super) fn read_field<T>(
    parameters: &ParamMap,
    field: &'static str,
) -> Result<Option<T>, CountCallError>
where
    T: DeserializeOwned,
{
    parameters.get(field).map_or(Ok(None), |raw| {
        serde_json::from_str::<Option<T>>(raw.get())
            .map_err(|source| CountCallError::Field { field, source })
    })
}

pub(super) fn transformed_body(
    request: &CountTokensHandlerRequest<'_>,
) -> Result<IndexMap<String, Json>, HandlerFailure> {
    let text = serde_json::to_string(&transform_request(request.request_data))
        .map_err(CountCallError::Encode)?;
    let body: IndexMap<String, Json> =
        serde_json::from_str(&text).map_err(CountCallError::Encode)?;
    tracing::debug!(
        subject = "transformed",
        value = %dumps_object(&body),
        "Transformed request: {}",
        dumps_object(&body)
    );
    Ok(body)
}

pub(super) fn reply_fields(
    response: &UpstreamResponse,
) -> Result<Option<CountResponse>, HandlerFailure> {
    let reply = decode_bedrock_count_reply(&response.body)?;
    tracing::debug!(
        subject = "reply",
        value = %dumps_object(&reply),
        "Bedrock response: {}",
        dumps_object(&reply)
    );
    let transformed = transform_response(&reply);
    let fields = IndexMap::from([("input_tokens".to_owned(), transformed.input_tokens)]);
    tracing::debug!(
        subject = "final",
        value = %dumps_object(&fields),
        "Final response: {}",
        dumps_object(&fields)
    );
    Ok(Some(fields))
}

pub(super) fn header_map(headers: &[(String, String)]) -> Result<HeaderMap, CountCallError> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let header_name =
            HeaderName::from_str(name).map_err(|source| CountCallError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let mut header_value =
            HeaderValue::from_str(value).map_err(|source| CountCallError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        if header_name == AUTHORIZATION {
            header_value.set_sensitive(true);
        }
        map.insert(header_name, header_value);
    }
    Ok(map)
}

fn header_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned()))
        })
        .collect()
}

// LiteLLM reads response.text, which decodes any bytes; betterlite refuses bytes that are not UTF-8
fn response_text(body: Vec<u8>) -> String {
    String::from_utf8(body)
        .unwrap_or_else(|error| format!("{} bytes that are not UTF-8", error.as_bytes().len()))
}

pub(super) fn upstream_record(
    response: &UpstreamResponse,
    endpoint: &str,
) -> Result<BedrockError, HeaderError> {
    let text = response_text(response.body.clone());
    let headers = header_pairs(&response.headers);
    log_upstream_failure(response.status, &text);
    let reply = ErrorReply {
        response: ErrorResponse {
            status_code: response.status,
            request: ErrorRequest {
                method: "POST".to_owned(),
                url: endpoint.to_owned(),
            },
        },
        headers: headers.clone(),
        body: ResponseBody::Read(text.clone()),
    };
    BedrockError::try_from(ErrorInput {
        status_code: response.status,
        message: text,
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: Some(ErrorHeaders::Http(headers)),
        response: Some(reply),
    })
}

fn log_upstream_failure(status: u16, text: &str) {
    if (400..=599).contains(&status) {
        // AsyncHTTPHandler raises for these statuses, so LiteLLM's HTTPStatusError branch catches them
        tracing::error!(
            subject = "HTTP error in CountTokens handler",
            detail = %text,
            "HTTP error in CountTokens handler: {text}"
        );
    } else {
        tracing::error!(
            subject = "AWS Bedrock error",
            detail = %text,
            "AWS Bedrock error: {text}"
        );
    }
}

pub(super) fn processing_record(source: &CountCallError) -> Result<BedrockError, HeaderError> {
    let message = format!("CountTokens processing error: {source}");
    tracing::error!(
        subject = "Error in CountTokens handler",
        detail = %source,
        "Error in CountTokens handler: {source}"
    );
    BedrockError::try_from(ErrorInput {
        status_code: 500,
        message,
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: None,
        response: None,
    })
}
