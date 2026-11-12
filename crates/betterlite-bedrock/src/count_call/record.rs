use std::str::FromStr as _;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::python_json::{dumps, dumps_object};
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
use super::reply::CountResponse;

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
) -> Result<IndexMap<String, Json>, CountCallError> {
    let text = serde_json::to_string(&transform_request(request.request_data))
        .map_err(CountCallError::Encode)?;
    let body: IndexMap<String, Json> =
        serde_json::from_str(&text).map_err(CountCallError::Encode)?;
    let value = dumps_object(&body);
    tracing::debug!(subject = "transformed", %value, "Transformed request: {value}");
    Ok(body)
}

pub(super) fn reply_fields(
    response: &UpstreamResponse,
) -> Result<Option<CountResponse>, CountCallError> {
    let parsed = serde_json::from_slice::<Json>(&response.body).map_err(CountCallError::Decode)?;
    let value = dumps(&parsed);
    tracing::debug!(subject = "reply", %value, "Bedrock response: {value}");
    let Json::Object(reply) = parsed else {
        return Err(CountCallError::NotAnObject);
    };
    let transformed = transform_response(&reply);
    let fields = IndexMap::from([("input_tokens".to_owned(), transformed.input_tokens)]);
    let finished = dumps_object(&fields);
    tracing::debug!(subject = "final", value = %finished, "Final response: {finished}");
    Ok(Some(fields))
}

pub(super) fn header_map(headers: &[(String, String)]) -> Result<HeaderMap, CountCallError> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let invalid = |source: http::Error| CountCallError::Header {
            name: name.clone(),
            source,
        };
        let header_name =
            HeaderName::from_str(name).map_err(|source| invalid(http::Error::from(source)))?;
        let mut header_value =
            HeaderValue::from_str(value).map_err(|source| invalid(http::Error::from(source)))?;
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

pub(super) fn upstream_record(
    response: &UpstreamResponse,
    endpoint: &str,
) -> Result<BedrockError, HeaderError> {
    // LiteLLM reads response.text, which decodes any bytes; betterlite refuses bytes that are not UTF-8
    let text = String::from_utf8(response.body.clone())
        .unwrap_or_else(|error| format!("{} bytes that are not UTF-8", error.as_bytes().len()));
    let headers = header_pairs(&response.headers);
    // AsyncHTTPHandler raises for every non-2xx reply, so LiteLLM's HTTPStatusError branch catches those
    let subject = if (200..300).contains(&response.status) {
        "AWS Bedrock error"
    } else {
        "HTTP error in CountTokens handler"
    };
    tracing::error!(subject, detail = %text, "{subject}: {text}");
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
