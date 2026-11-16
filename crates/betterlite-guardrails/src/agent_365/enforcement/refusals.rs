use std::time::Duration;

use super::record::{VerdictRecord, record_verdict};
use crate::agent_365::enforcement_types::{UnavailableDetail, Verdict};
use crate::agent_365_tokens::Agent365Guardrail;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_metadata::RequestData;

pub(super) const CALLER_FAULT_ERROR: &str = "Agent 365 guardrail rejected the tool call";
pub(super) const UNAVAILABLE_ERROR: &str = "Agent 365 guardrail could not authorize the tool call";
pub(super) const UNAVAILABLE_STATUS: u16 = 503;

// The refusals LiteLLM raises before any verdict, which differ only in their record, status, error and message
pub(super) enum Refusal<'call> {
    CallerFault {
        status: u16,
        tool_name: &'call str,
        reason: &'call str,
    },
    Throttled {
        tool_name: &'call str,
        reason: &'call str,
        latency: Option<Duration>,
    },
}

impl<'call> Refusal<'call> {
    fn names(&self) -> (&'call str, &'call str) {
        match *self {
            Self::CallerFault {
                tool_name, reason, ..
            }
            | Self::Throttled {
                tool_name, reason, ..
            } => (tool_name, reason),
        }
    }

    fn message(&self) -> String {
        match *self {
            Self::CallerFault {
                tool_name, reason, ..
            } => format!("Tool call '{tool_name}' was blocked because {reason}."),
            Self::Throttled {
                tool_name, reason, ..
            } => format!(
                "Tool call '{tool_name}' was blocked because {reason}; throttled evaluations block regardless of unreachable_fallback."
            ),
        }
    }
}

// LiteLLM's _handle_caller_fault and _handle_throttled: record the refusal, then raise its detail
pub(super) fn refuse(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    refusal: &Refusal<'_>,
) -> Result<(), GuardrailFailure> {
    let (verdict, latency, status, error) = match *refusal {
        Refusal::CallerFault { status, .. } => {
            (Verdict::Rejected, None, status, CALLER_FAULT_ERROR)
        }
        Refusal::Throttled { latency, .. } => (
            Verdict::Throttled,
            latency,
            UNAVAILABLE_STATUS,
            UNAVAILABLE_ERROR,
        ),
    };
    let (tool_name, reason) = refusal.names();
    record_verdict(
        guardrail,
        data,
        &VerdictRecord {
            verdict,
            defender_status: None,
            correlation_id: None,
            latency,
            reason: Some(reason),
        },
    )?;
    Err(refusal_failure(status, error, refusal.message(), tool_name))
}

// The detail of a refusal: the short error, the message and the tool the call named
pub(super) fn refusal_failure(
    status: u16,
    error: &str,
    message: String,
    tool_name: &str,
) -> GuardrailFailure {
    let detail = UnavailableDetail {
        error: error.to_owned(),
        message,
        tool: tool_name.to_owned(),
    };
    GuardrailFailure::HttpObject {
        status,
        detail: detail.detail(),
    }
}
