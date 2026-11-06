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

// _get_status_fields aggregates every entry: an earlier passing guardrail must not hide a later block.
#[must_use]
pub fn get_status_fields(
    status: LoggingStatus,
    guardrail_information: Option<&[GuardrailInformation]>,
) -> StatusFields {
    let mut guardrail_status = GuardrailStatus::NotRun;
    for entry in guardrail_information.unwrap_or_default() {
        let Some(raw) = entry.guardrail_status.as_deref() else {
            continue;
        };
        guardrail_status = higher_severity(guardrail_status, map_status(raw));
    }
    StatusFields {
        llm_api_status: status,
        guardrail_status,
    }
}

// LiteLLM's GUARDRAIL_STATUS_MAP, including the legacy blocked and failure values; anything else is not_run.
fn map_status(raw: &str) -> GuardrailStatus {
    match raw {
        "success" => GuardrailStatus::Success,
        "guardrail_flagged" => GuardrailStatus::GuardrailFlagged,
        "blocked" | "guardrail_intervened" => GuardrailStatus::GuardrailIntervened,
        "failure" | "guardrail_failed_to_respond" => GuardrailStatus::GuardrailFailedToRespond,
        _ => GuardrailStatus::NotRun,
    }
}

const fn higher_severity(current: GuardrailStatus, candidate: GuardrailStatus) -> GuardrailStatus {
    if severity(candidate) > severity(current) {
        candidate
    } else {
        current
    }
}

// LiteLLM's GUARDRAIL_STATUS_SEVERITY, least severe first.
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
