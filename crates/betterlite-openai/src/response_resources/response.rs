use betterlite_models::call_helpers::headers::{
    AdditionalHeaders, HeaderSource, process_response_headers,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::de::DeserializeOwned;

use crate::api::responses_objects::ResponsesApiResponse;
use crate::api::responses_shapes::wire::{
    ShapeDecodeError, WireDecodeLimits, decode_responses_shapes,
};
use crate::error::{OpenAiError, ProviderError, ProviderErrorParts};
use crate::response_events::RawResponse;

use super::error::ResponseResourceError;

// a retrieved response can carry a base64 image over 1 MiB, so the cap matches the crate's other large readers
pub(super) const MAX_BODY_BYTES: usize = 0x400_0000;

const DECODE_LIMITS: WireDecodeLimits = WireDecodeLimits {
    max_json_bytes: MAX_BODY_BYTES,
    max_event_bytes: MAX_BODY_BYTES,
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
        Err(ResponseResourceError::Decode { source }) => match source {
            ShapeDecodeError::Json(_) => Err(ResponseResourceError::Provider(
                OpenAiError::Provider(Box::new(ProviderError::from(ProviderErrorParts {
                    status_code: reply.status,
                    message: reply.text.to_owned(),
                    headers: None,
                    request: None,
                    response: None,
                    body: None,
                    status_code_is_synthesized: false,
                }))),
            )),
            ShapeDecodeError::JsonTooLarge
            | ShapeDecodeError::TooManyItems
            | ShapeDecodeError::TooDeep => Err(ResponseResourceError::Decode { source }),
        },
        Err(other) => Err(other),
    }
}

pub fn decode_record<T>(reply: &RawResponse<'_>) -> Result<T, ResponseResourceError>
where
    T: DeserializeOwned,
{
    let decoded = json_body(reply)?;
    let value =
        serde_json::to_value(decoded).map_err(|source| ResponseResourceError::Encode { source })?;
    serde_json::from_value(value).map_err(|source| ResponseResourceError::Record { source })
}

pub fn response_reply(reply: &RawResponse<'_>) -> Result<TypedReply, ResponseResourceError> {
    let response: ResponsesApiResponse = decode_record(reply)?;
    Ok(TypedReply {
        response,
        headers: reply.headers.clone(),
        additional_headers: process_response_headers(HeaderSource::Upstream(reply.headers)),
    })
}
