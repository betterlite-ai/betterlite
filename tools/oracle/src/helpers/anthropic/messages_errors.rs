use serde::Deserialize;
use serde_json::to_value;

use betterlite_anthropic::api::messages_errors::{
    AnthropicErrorSseFrame, DecodedMessagesErrors, WireDecodeLimits, decode_messages_errors,
    transform_messages_error,
};

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub action: String,
    #[serde(default)]
    pub status: u16,
    #[serde(default)]
    pub raw_message: String,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub call_id: Option<String>,
    #[serde(default)]
    pub bytes: Vec<u8>,
    #[serde(default)]
    pub limits: Option<WireDecodeLimitsInput>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WireDecodeLimitsInput {
    pub max_json_bytes: usize,
    pub max_event_bytes: usize,
    pub max_items: usize,
}

impl From<&WireDecodeLimitsInput> for WireDecodeLimits {
    fn from(limits: &WireDecodeLimitsInput) -> Self {
        Self {
            max_json_bytes: limits.max_json_bytes,
            max_event_bytes: limits.max_event_bytes,
            max_items: limits.max_items,
        }
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.action.as_str() {
            "transform" => to_value(transform_messages_error(
                input.status,
                &input.raw_message,
                input.request_id.as_deref(),
            ))
            .map_err(serialization_error),
            "frame" => {
                let frame = AnthropicErrorSseFrame::new(
                    input.status,
                    transform_messages_error(input.status, &input.raw_message, None),
                );
                to_value(FrameOutput {
                    status: frame.status_code,
                    body: frame.body().to_owned(),
                    json_body: frame.json_body(input.call_id.as_deref()),
                })
                .map_err(serialization_error)
            }
            "wire_roundtrip" => {
                let limits = input.limits.as_ref().map_or(
                    WireDecodeLimits {
                        max_json_bytes: input.bytes.len(),
                        max_event_bytes: input.bytes.len(),
                        max_items: usize::MAX,
                    },
                    WireDecodeLimits::from,
                );
                let decoded = decode_messages_errors(&input.bytes, limits).map_err(|error| {
                    Box::new(crate::helpers::HelperError {
                        class: "WireDecodeError".to_owned(),
                        status: None,
                        error_type: None,
                        param: None,
                        code: None,
                        message: error.to_string(),
                    })
                })?;
                match decoded {
                    DecodedMessagesErrors::Error(response) => to_value(response),
                    DecodedMessagesErrors::Response(response) => to_value(response),
                }
                .map_err(serialization_error)
            }
            _ => Err(Box::new(crate::helpers::HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: "unknown messages_errors action".to_owned(),
            })),
        }
    }
}

fn serialization_error(error: serde_json::Error) -> Box<crate::helpers::HelperError> {
    let helper_error = Box::new(crate::helpers::HelperError {
        class: "SerializationError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    });
    helper_error
}

#[derive(serde::Serialize)]
struct FrameOutput {
    status: u16,
    body: String,
    json_body: betterlite_anthropic::api::messages_errors::AnthropicErrorResponse,
}
