use betterlite_anthropic::api::messages_errors::AnthropicExceptionMapping;
use betterlite_anthropic::api::messages_errors::decode::{
    WireDecodeError, WireDecodeLimits, decode_messages_errors,
};
use betterlite_anthropic::api::messages_errors::frame::anthropic_error_sse_frame;
use betterlite_anthropic::api::messages_errors::wire::AnthropicMetadata;
use serde::Deserialize;
use serde_json::to_value;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Input {
    One(Action),
    Many { actions: Vec<Action> },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum Action {
    Transform {
        status: u16,
        raw_message: String,
        request_id: Option<String>,
    },
    Frame {
        status: u16,
        raw_message: String,
        call_id: Option<String>,
    },
    WireRoundtrip {
        json: String,
        #[serde(default)]
        metadata: bool,
    },
}

const LIMITS: WireDecodeLimits = WireDecodeLimits {
    max_json_bytes: 0x0001_0000,
    max_event_bytes: 0x0001_0000,
    max_items: 4096,
};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::One(action) => run_action(action),
            Input::Many { actions } => {
                let results = actions
                    .iter()
                    .map(run_action)
                    .collect::<Result<Vec<_>, _>>()?;
                to_value(results).map_err(|error| helper_error(&WireDecodeError::Json(error)))
            }
        }
    }
}

fn run_action(action: &Action) -> crate::helpers::HelperResult {
    let result = match action {
        Action::Transform {
            status,
            raw_message,
            request_id,
        } => {
            let response = AnthropicExceptionMapping::transform_to_anthropic_error(
                *status,
                raw_message,
                request_id.as_deref(),
                LIMITS,
            )
            .map_err(|error| helper_error(&error))?;
            to_value(response)
        }
        Action::Frame {
            status,
            raw_message,
            call_id,
        } => return frame_output(*status, raw_message, call_id.as_deref()),
        Action::WireRoundtrip {
            json,
            metadata: true,
        } => {
            let metadata: AnthropicMetadata = serde_json::from_str(json)
                .map_err(|error| helper_error(&WireDecodeError::Json(error)))?;
            to_value(metadata)
        }
        Action::WireRoundtrip {
            json,
            metadata: false,
        } => {
            let decoded = decode_messages_errors(json.as_bytes(), LIMITS)
                .map_err(|error| helper_error(&error))?;
            to_value(decoded)
        }
    };
    result.map_err(|error| helper_error(&WireDecodeError::Json(error)))
}

fn frame_output(
    status: u16,
    raw_message: &str,
    call_id: Option<&str>,
) -> crate::helpers::HelperResult {
    let frame = anthropic_error_sse_frame(status, raw_message, LIMITS)
        .map_err(|error| helper_error(&error))?;
    let json_body = frame
        .json_body(call_id)
        .map_err(|error| helper_error(&error))?;
    to_value(serde_json::json!({"status": frame.status_code, "body": frame.body, "json_body": json_body}))
        .map_err(|error| helper_error(&WireDecodeError::Json(error)))
}

fn helper_error(error: &WireDecodeError) -> Box<crate::helpers::HelperError> {
    let message = error.to_string();
    Box::new(crate::helpers::HelperError {
        class: "WireDecodeError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
