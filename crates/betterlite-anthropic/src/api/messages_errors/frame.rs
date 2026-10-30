use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use serde_json::value::to_raw_value;

use super::decode::{WireDecodeError, WireDecodeLimits};
use super::wire::{ErrorField, ErrorRecord, TextField};
use super::{AnthropicErrorSseFrame, AnthropicExceptionMapping};

impl AnthropicErrorSseFrame {
    pub fn new(status_code: u16, error_response: ErrorRecord) -> Result<Self, WireDecodeError> {
        let raw = to_raw_value(&error_response)?;
        let document: Json = serde_json::from_str(raw.get())?;
        let body = format!("event: error\ndata: {}\n\n", dumps(&document));
        Ok(Self {
            status_code,
            error_response,
            body,
        })
    }

    pub fn json_body(&self, call_id: Option<&str>) -> Result<ErrorRecord, WireDecodeError> {
        let mut response = self.error_response.clone();
        if let Some(identifier) = call_id
            && let ErrorField::Known(error) = &mut response.head.error
        {
            error.head.litellm_call_id = TextField::Known(identifier.to_owned());
            error
                .fields
                .insert("litellm_call_id".to_owned(), to_raw_value(identifier)?);
            response
                .fields
                .insert("error".to_owned(), to_raw_value(error)?);
        }
        Ok(response)
    }
}

pub fn anthropic_error_sse_frame(
    status: u16,
    raw_message: &str,
    limits: WireDecodeLimits,
) -> Result<AnthropicErrorSseFrame, WireDecodeError> {
    let response =
        AnthropicExceptionMapping::transform_to_anthropic_error(status, raw_message, None, limits)?;
    AnthropicErrorSseFrame::new(status, response)
}
