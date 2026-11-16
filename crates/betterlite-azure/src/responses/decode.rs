use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::response_resources::error::ResponseResourceError;
use betterlite_openai::response_resources::response::decode_response_resource;
use betterlite_prompts::arguments::json::Json;

use super::error::ResponsesError;

// The byte boundary the cancel path reads: response JSON bytes, then the response record
pub fn decode_azure_response_resource(
    bytes: &[u8],
) -> Result<ResponsesApiResponse, ResponsesError> {
    let decoded = decode_response_resource(bytes)?;
    Ok(record(&decoded)?)
}

pub fn record(decoded: &Json) -> Result<ResponsesApiResponse, ResponsesError> {
    let value =
        serde_json::to_value(decoded).map_err(|source| ResponseResourceError::Encode { source })?;
    serde_json::from_value(value).map_err(ResponsesError::Record)
}
