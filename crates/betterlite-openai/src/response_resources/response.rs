use betterlite_models::call_helpers::headers::{
    AdditionalHeaders, HeaderSource, process_response_headers,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::responses_objects::ResponsesApiResponse;
use crate::api::responses_shapes::DeleteResponseResult;
use crate::api::responses_shapes::wire::{WireDecodeLimits, decode_responses_shapes};
use crate::error::{OpenAiError, ProviderError, ProviderErrorParts};
use crate::response_events::RawResponse;

use super::error::ResponseResourceError;

const DECODE_LIMITS: WireDecodeLimits = WireDecodeLimits {
    max_json_bytes: 0x0010_0000,
    max_event_bytes: 0x0010_0000,
    max_items: 0x4000,
};

#[derive(Debug)]
pub struct TypedReply {
    pub response: ResponsesApiResponse,
    pub headers: IndexMap<String, String>,
    pub additional_headers: AdditionalHeaders,
}

pub fn decode_response_resource(bytes: &[u8]) -> Result<Json, ResponseResourceError> {
    match decode_responses_shapes(bytes, DECODE_LIMITS) {
        Ok(decoded) => Ok(decoded),
        Err(source) => Err(ResponseResourceError::Decode { source }),
    }
}

pub fn json_body(reply: &RawResponse<'_>) -> Result<Json, ResponseResourceError> {
    match decode_response_resource(reply.text.as_bytes()) {
        Ok(decoded) => Ok(decoded),
        Err(_decode) => Err(ResponseResourceError::Provider(OpenAiError::Provider(
            Box::new(ProviderError::from(ProviderErrorParts {
                status_code: reply.status,
                message: reply.text.to_owned(),
                headers: None,
                request: None,
                response: None,
                body: None,
                status_code_is_synthesized: false,
            })),
        ))),
    }
}

pub fn delete_response(
    reply: &RawResponse<'_>,
) -> Result<DeleteResponseResult, ResponseResourceError> {
    let decoded = json_body(reply)?;
    let value =
        serde_json::to_value(decoded).map_err(|source| ResponseResourceError::Encode { source })?;
    let result =
        serde_json::from_value(value).map_err(|source| ResponseResourceError::Record { source })?;
    Ok(result)
}

pub fn response_reply(reply: &RawResponse<'_>) -> Result<TypedReply, ResponseResourceError> {
    let decoded = json_body(reply)?;
    let value =
        serde_json::to_value(decoded).map_err(|source| ResponseResourceError::Encode { source })?;
    let response: ResponsesApiResponse =
        serde_json::from_value(value).map_err(|source| ResponseResourceError::Record { source })?;
    Ok(TypedReply {
        response,
        headers: reply.headers.clone(),
        additional_headers: process_response_headers(HeaderSource::Upstream(reply.headers)),
    })
}
