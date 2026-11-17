use std::sync::Arc;
use std::time::{Duration, SystemTime};

use betterlite_core::api::responses_stream_errors::ResponsesStreamErrorState;
use betterlite_core::api::responses_stream_errors::failure::FailureCause;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

const FIXED_TIME: u64 = 1_800_000_000;
const FIXED_UUID: u128 = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;
const FRAME_PREFIX: &str = "event: response.failed\ndata: ";
const FRAME_SUFFIX: &str = "\n\n";
const MASKED_ID: &str = "resp_00000000000000000000000000000000";
const MASKED_TIME: i64 = -999_999_999;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Redaction {
    Enabled,
    Disabled,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    ObserveChunk { chunk: Json },
    MarkEmitted { frame: String },
    FormatFailure { failure: FailureCause },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    redaction: Redaction,
    steps: Vec<Step>,
}

#[derive(serde::Serialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum StepResult {
    ObserveChunk,
    MarkEmitted { frame: String },
    FormatFailure { text: Option<String> },
}

#[derive(serde::Serialize)]
struct StateView {
    response_id: Option<String>,
    model: Option<String>,
    created_at: Option<i64>,
    sequence_number: i64,
    terminal_emitted: bool,
}

#[derive(serde::Serialize)]
struct Output {
    results: Vec<StepResult>,
    state: StateView,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut state = build_state(input.redaction)?;
        let mut results = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            results.push(match step {
                Step::ObserveChunk { chunk } => {
                    state
                        .observe_chunk(chunk)
                        .map_err(|error| helper_error(&error.to_string()))?;
                    StepResult::ObserveChunk
                }
                Step::MarkEmitted { frame } => StepResult::MarkEmitted {
                    frame: state.mark_emitted(frame.clone()),
                },
                Step::FormatFailure { failure } => {
                    let id_committed = state
                        .response_id
                        .as_deref()
                        .is_some_and(|id| !id.is_empty());
                    let time_committed = state.created_at.is_some();
                    let text = state
                        .format_failure(failure)
                        .map(|frame| mask_frame(&frame, id_committed, time_committed));
                    StepResult::FormatFailure { text }
                }
            });
        }
        let output = Output {
            results,
            state: StateView {
                response_id: state.response_id,
                model: state.model,
                created_at: state.created_at,
                sequence_number: state.sequence_number,
                terminal_emitted: state.terminal_emitted,
            },
        };
        serde_json::to_value(output).map_err(|error| helper_error(&error.to_string()))
    }
}

fn build_state(redaction: Redaction) -> Result<ResponsesStreamErrorState, Box<HelperError>> {
    let redactor = match redaction {
        Redaction::Enabled => Some(Arc::new(
            Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
                .map_err(|error| helper_error(&error.to_string()))?,
        )),
        Redaction::Disabled => None,
    };
    let Some(fixed_time) = SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(FIXED_TIME))
    else {
        return Err(helper_error("the fixed oracle time is not representable"));
    };
    let now: Arc<dyn Fn() -> SystemTime + Send + Sync> = Arc::new(move || fixed_time);
    let next_id: Arc<dyn Fn() -> Uuid + Send + Sync> = Arc::new(|| Uuid::from_u128(FIXED_UUID));
    Ok(ResponsesStreamErrorState::new(now, next_id, redactor))
}

fn mask_frame(frame: &str, id_committed: bool, time_committed: bool) -> String {
    let Some(data) = frame
        .strip_prefix(FRAME_PREFIX)
        .and_then(|rest| rest.strip_suffix(FRAME_SUFFIX))
    else {
        return frame.to_owned();
    };
    let Ok(event) = serde_json::from_str::<Value>(data) else {
        return frame.to_owned();
    };
    let Some(response) = event.get("response") else {
        return frame.to_owned();
    };
    let mut masked = frame.to_owned();
    if !id_committed && let Some(id) = response.get("id").and_then(Value::as_str) {
        masked = masked.replace(id, MASKED_ID);
    }
    if !time_committed && let Some(created_at) = response.get("created_at").and_then(Value::as_i64)
    {
        masked = masked.replace(&created_at.to_string(), &MASKED_TIME.to_string());
    }
    masked
}

fn helper_error(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ResponsesStreamError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
