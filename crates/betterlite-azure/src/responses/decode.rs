use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::response_events::RawResponse;
use betterlite_openai::response_resources::error::ResponseResourceError;
use betterlite_openai::response_resources::response::{decode_response_resource, json_body};

use super::error::ResponsesError;

// The outside-byte decoder the fuzz target and the corpus replay read: response JSON bytes, then the record
pub fn decode_azure_response_resource(
    bytes: &[u8],
) -> Result<ResponsesApiResponse, ResponsesError> {
    decode_response_resource(bytes)
        .map_err(ResponsesError::from)
        .and_then(|decoded| record(&decoded))
}

// LiteLLM's cancel transform: a body that is not JSON keeps the reply's raw text and status, then the record validates
pub fn cancel_reply(reply: &RawResponse<'_>) -> Result<ResponsesApiResponse, ResponsesError> {
    json_body(reply)
        .map_err(ResponsesError::from)
        .and_then(|body| record(&body))
}

fn record(decoded: &Json) -> Result<ResponsesApiResponse, ResponsesError> {
    let value =
        serde_json::to_value(decoded).map_err(|source| ResponseResourceError::Encode { source })?;
    serde_json::from_value(value).map_err(ResponsesError::Record)
}
