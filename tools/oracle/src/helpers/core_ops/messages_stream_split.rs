mod projection;
mod source;
mod split_step;

use betterlite_anthropic::api::messages_errors::decode::WireDecodeLimits;
use betterlite_core::api::messages_stream_split::delta::StreamingDeltaType;
use betterlite_core::api::messages_stream_split::errors::{
    StreamFailure, error_status_and_message, mid_stream_error_sse_event,
};
use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, OrHelperError as _};

pub(crate) struct Op;

const LIMITS: WireDecodeLimits = WireDecodeLimits {
    max_json_bytes: 0x0010_0000,
    max_event_bytes: 0x0010_0000,
    max_items: 1_000,
};

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Step {
    Split(SplitStep),
    ErrorStatusAndMessage { failure: FailureInput },
    MidStreamErrorSseEvent { failure: FailureInput },
    DeltaPayloadField { delta_type: StreamingDeltaType },
}

#[derive(Debug, Deserialize)]
pub(super) struct SplitStep {
    #[serde(default)]
    pub(super) mode: Mode,
    pub(super) splitters: usize,
    pub(super) rounds: usize,
    #[serde(default)]
    pub(super) chunks: Vec<StreamChunk>,
    #[serde(default)]
    pub(super) chunks_attr: Option<Vec<StreamChunk>>,
    #[serde(default)]
    pub(super) messages_attr: Option<Vec<Json>>,
    #[serde(default)]
    pub(super) failure: Option<FailureInput>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Mode {
    #[default]
    Sync,
    Async,
}

#[derive(Debug, Deserialize)]
pub(super) struct FailureInput {
    pub(super) kind: FailureKind,
    #[serde(default)]
    pub(super) status_code: Option<u16>,
    pub(super) message: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum FailureKind {
    Reported,
    Unclassified,
}

pub(super) fn stream_failure(input: &FailureInput) -> Result<StreamFailure, Box<HelperError>> {
    match input.kind {
        FailureKind::Reported => {
            let Some(status_code) = input.status_code else {
                return Err("a reported failure names its status_code").or_value_error();
            };
            Ok(StreamFailure::Reported {
                status_code,
                message: input.message.clone(),
            })
        }
        FailureKind::Unclassified => Ok(StreamFailure::Unclassified {
            text: input.message.clone(),
        }),
    }
}

fn answer(step: &Step) -> Result<Value, Box<HelperError>> {
    match step {
        Step::Split(split) => serde_json::to_value(split_step::run_split(split)?).or_value_error(),
        Step::ErrorStatusAndMessage { failure } => {
            let (status, message) = error_status_and_message(&stream_failure(failure)?);
            Ok(json!({"status": status, "message": message}))
        }
        Step::MidStreamErrorSseEvent { failure } => {
            let frame = mid_stream_error_sse_event(&stream_failure(failure)?, LIMITS);
            Ok(Value::String(frame.or_value_error()?))
        }
        Step::DeltaPayloadField { delta_type } => {
            Ok(Value::String(delta_type.payload_field().to_owned()))
        }
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut answers = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            answers.push(answer(step)?);
        }
        Ok(Value::Array(answers))
    }
}
