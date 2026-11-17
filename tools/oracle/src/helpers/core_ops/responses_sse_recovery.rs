use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_sse_recovery::{decode_recovery_line, recover_recorded_output};
use betterlite_models::decisions::Json;
use serde_json::json;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    action: Action,
    #[serde(default)]
    lines: Vec<String>,
    #[serde(default)]
    recordings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    RecoveryLineFilter,
    RecoverRecordedOutput,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let limits = ApiLimits::default();
        match input.action {
            Action::RecoveryLineFilter => Ok(json!(decoded_lines(&input.lines, &limits)?)),
            Action::RecoverRecordedOutput => {
                Ok(json!(recovered_recordings(&input.recordings, &limits)?))
            }
        }
    }
}

fn decoded_lines(
    lines: &[String],
    limits: &ApiLimits,
) -> Result<Vec<Option<Json>>, Box<HelperError>> {
    lines
        .iter()
        .map(|line| {
            decode_recovery_line(line.as_bytes(), limits)
                .map(|parsed| parsed.map(Json::Object))
                .map_err(|error| helper_error("ValueError", error.to_string()))
        })
        .collect()
}

fn recovered_recordings(
    recordings: &[String],
    limits: &ApiLimits,
) -> Result<Vec<Vec<Json>>, Box<HelperError>> {
    recordings
        .iter()
        .map(|recording| {
            recover_recorded_output(recording.as_bytes(), limits)
                .map_err(|error| helper_error("ValueError", error.to_string()))
        })
        .collect()
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
