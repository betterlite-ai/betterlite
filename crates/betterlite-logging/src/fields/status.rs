use serde::{Deserialize, Serialize};

use crate::fields::payload::records::GuardrailInformation;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoggingStatus {
    Success,
    Failure,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailStatus {
    Success,
    GuardrailFlagged,
    GuardrailIntervened,
    GuardrailFailedToRespond,
    NotRun,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StatusFields {
    pub llm_api_status: LoggingStatus,
    pub guardrail_status: GuardrailStatus,
}

#[must_use]
pub fn get_status_fields(
    status: LoggingStatus,
    guardrail_information: Option<&[GuardrailInformation]>,
) -> StatusFields {
    let guardrail_status = guardrail_information
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| entry.guardrail_status.as_ref().and_then(Option::as_deref))
        .map(map_status)
        .max_by_key(|mapped| severity(*mapped))
        .unwrap_or(GuardrailStatus::NotRun);
    StatusFields {
        llm_api_status: status,
        guardrail_status,
    }
}

fn map_status(raw: &str) -> GuardrailStatus {
    match raw {
        "success" => GuardrailStatus::Success,
        "guardrail_flagged" => GuardrailStatus::GuardrailFlagged,
        "blocked" | "guardrail_intervened" => GuardrailStatus::GuardrailIntervened,
        "failure" | "guardrail_failed_to_respond" => GuardrailStatus::GuardrailFailedToRespond,
        _ => GuardrailStatus::NotRun,
    }
}

const fn severity(status: GuardrailStatus) -> u8 {
    match status {
        GuardrailStatus::NotRun => 0,
        GuardrailStatus::Success => 1,
        GuardrailStatus::GuardrailFlagged => 2,
        GuardrailStatus::GuardrailFailedToRespond => 3,
        GuardrailStatus::GuardrailIntervened => 4,
    }
}

mod tests;
