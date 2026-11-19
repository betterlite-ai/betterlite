mod tests;

use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_server::shared_stream::failure::failure_exception;
use betterlite_server::shared_stream::framing::ChunkFraming as _;
use betterlite_server::shared_stream::generator::DEFAULT_LIMITS;
use betterlite_server::shared_stream::sse::{Emitted, SseFraming};
use betterlite_server::shared_stream::tail::{
    SSE_STREAM_START_TAIL, advance_sse_tail, seal_open_sse_frame,
};
use betterlite_server::sse_data_generator::types::{
    FailureKind, FailureParam, FailureType, StreamFailure,
};
use betterlite_server::stream_chunks::restamp::should_return_raw_model_name;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    kind: Kind,
    #[serde(default)]
    writes: Vec<String>,
    #[serde(default)]
    request_data: Option<JsonObject>,
    #[serde(default)]
    failure: Option<FailureSpec>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Seal,
    RawModel,
    Failure,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FailureSpec {
    message: String,
    #[serde(default, rename = "type")]
    error_type: Option<String>,
    #[serde(default)]
    param: Option<String>,
    #[serde(default)]
    status_code: Option<u16>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.kind {
            Kind::Seal => Ok(seal_value(&input.writes)),
            Kind::RawModel => {
                let Some(request_data) = input.request_data.as_ref() else {
                    return Err(missing("request_data"));
                };
                Ok(json!({ "raw": should_return_raw_model_name(request_data) }))
            }
            Kind::Failure => {
                let Some(spec) = input.failure.as_ref() else {
                    return Err(missing("failure"));
                };
                failure_value(spec)
            }
        }
    }
}

fn missing(name: &str) -> Box<HelperError> {
    helper_error(format!("the b9720 {name} field is required"))
}

fn helper_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}

// the tail after each write and the seal it asks for, both folded from LiteLLM's start tail
fn seal_value(writes: &[String]) -> Value {
    let mut tail = SSE_STREAM_START_TAIL.to_vec();
    let mut tails = vec![json!(hex::encode(&tail))];
    let mut seals = vec![json!(seal_open_sse_frame(&tail))];
    for write in writes {
        tail = advance_sse_tail(&tail, write.as_bytes());
        tails.push(json!(hex::encode(&tail)));
        seals.push(json!(seal_open_sse_frame(&tail)));
    }
    json!({ "tails": tails, "seals": seals })
}

fn failure_value(spec: &FailureSpec) -> HelperResult {
    let failure = StreamFailure {
        message: spec.message.clone(),
        error_type: match &spec.error_type {
            Some(text) => FailureType::Text(text.clone()),
            None => FailureType::Missing,
        },
        param: match &spec.param {
            Some(text) => FailureParam::Text(text.clone()),
            None => FailureParam::Missing,
        },
        status_code: spec.status_code,
        code: None,
        body: None,
        kind: FailureKind::Other,
    };
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_config_error()?;
    let exception = failure_exception(&failure, Some(&redactor));
    let mut framing = SseFraming::new(None, DEFAULT_LIMITS);
    let frame = framing.frame_failure(&exception).map_err(|abort| {
        helper_error(format!(
            "the error frame could not be built: {} with {}",
            abort.status, abort.detail
        ))
    })?;
    let (status, body) = match frame {
        Emitted::AnthropicError(frame) => (frame.status_code, frame.body),
        Emitted::Frame(_) => {
            return Err(helper_error(String::from(
                "the start tail must seal an Anthropic error frame",
            )));
        }
    };
    Ok(json!({
        "error": {
            "message": exception.message,
            "type": exception.error_type,
            "param": exception.param,
            "code": exception.code,
        },
        "frame_status": status,
        "frame_body": body,
    }))
}
