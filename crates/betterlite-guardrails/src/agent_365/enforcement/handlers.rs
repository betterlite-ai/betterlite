use std::time::Duration;

use betterlite_config::sections::guardrail_params::base::UnreachableFallback;

use super::Step;
use super::record::{VerdictRecord, record_verdict};
use crate::agent_365::enforcement_types::{
    BlockedDetail, EvaluateVerdict, UnavailableDetail, Verdict, decode_evaluate_response,
};
use crate::agent_365_tokens::Agent365Guardrail;
use crate::agent_365_tokens::poster::PostReply;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_metadata::RequestData;

pub(super) const CALLER_FAULT_ERROR: &str = "Agent 365 guardrail rejected the tool call";
pub(super) const UNAVAILABLE_ERROR: &str = "Agent 365 guardrail could not authorize the tool call";
pub(super) const REJECTED_ERROR: &str = "Agent 365 rejected the tool evaluation request";
pub(super) const BLOCKED_ERROR: &str = "Blocked by Microsoft Defender";
const UNAVAILABLE_STATUS: u16 = 503;
const REJECTED_STATUS: u16 = 400;
const MAX_ERROR_TEXT_CHARS: usize = 512;

// The arguments of _handle_evaluate_error; Continue means the verdict still has to be enforced
pub(super) struct EvaluateError<'reply> {
    pub tool_name: &'reply str,
    pub assertion: &'reply str,
    pub reply: &'reply PostReply,
    pub latency: Duration,
}

// LiteLLM's _handle_caller_fault
pub(super) fn caller_fault(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    tool_name: &str,
    status: u16,
    reason: &str,
) -> Result<(), GuardrailFailure> {
    write_record(
        guardrail,
        data,
        &VerdictRecord {
            verdict: Verdict::Rejected,
            defender_status: None,
            correlation_id: None,
            latency: None,
            reason: Some(reason),
        },
    )?;
    let detail = UnavailableDetail {
        error: CALLER_FAULT_ERROR.to_owned(),
        message: format!("Tool call '{tool_name}' was blocked because {reason}."),
        tool: tool_name.to_owned(),
    };
    Err(GuardrailFailure::HttpObject {
        status,
        detail: detail.detail(),
    })
}

// LiteLLM's _handle_throttled
pub(super) fn throttled(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    tool_name: &str,
    reason: &str,
    latency: Option<Duration>,
) -> Result<(), GuardrailFailure> {
    write_record(
        guardrail,
        data,
        &VerdictRecord {
            verdict: Verdict::Throttled,
            defender_status: None,
            correlation_id: None,
            latency,
            reason: Some(reason),
        },
    )?;
    let detail = UnavailableDetail {
        error: UNAVAILABLE_ERROR.to_owned(),
        message: format!(
            "Tool call '{tool_name}' was blocked because {reason}; throttled evaluations block regardless of unreachable_fallback."
        ),
        tool: tool_name.to_owned(),
    };
    Err(GuardrailFailure::HttpObject {
        status: UNAVAILABLE_STATUS,
        detail: detail.detail(),
    })
}

// LiteLLM's _handle_unavailable
pub(super) fn unavailable(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    tool_name: &str,
    record: &VerdictRecord<'_>,
) -> Result<(), GuardrailFailure> {
    let reason = record.reason.unwrap_or_default();
    if guardrail.settings.unreachable_fallback == UnreachableFallback::FailOpen {
        return allow_unscanned(guardrail, data, tool_name, record);
    }
    write_record(guardrail, data, record)?;
    let detail = UnavailableDetail {
        error: UNAVAILABLE_ERROR.to_owned(),
        message: format!(
            "Tool call '{tool_name}' was blocked because {reason} and unreachable_fallback is 'fail_closed'."
        ),
        tool: tool_name.to_owned(),
    };
    Err(GuardrailFailure::HttpObject {
        status: UNAVAILABLE_STATUS,
        detail: detail.detail(),
    })
}

fn allow_unscanned(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    tool_name: &str,
    record: &VerdictRecord<'_>,
) -> Result<(), GuardrailFailure> {
    let reason = record.reason.unwrap_or_default();
    let name = &guardrail.settings.guardrail_name;
    tracing::error!(
        "Agent 365 guardrail ({name}): {reason}; unreachable_fallback='fail_open', allowing tool call '{tool_name}' unscanned"
    );
    write_record(
        guardrail,
        data,
        &VerdictRecord {
            verdict: Verdict::Unscanned,
            defender_status: record.defender_status,
            correlation_id: record.correlation_id,
            latency: record.latency,
            reason: record.reason,
        },
    )
}

// Runs _handle_evaluate_error and, when it lets the call through, _enforce_verdict
pub(super) fn enforce_reply(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    error: &EvaluateError<'_>,
) -> Result<(), GuardrailFailure> {
    let step = handle_evaluate_error(guardrail, data, error)?;
    if matches!(step, Step::Continue) {
        let _done = enforce_verdict(guardrail, data, error.tool_name, error.reply, error.latency)?;
    }
    Ok(())
}

// LiteLLM's _handle_evaluate_error; 408 and 429 are checked before the rest of the 4xx range
fn handle_evaluate_error(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    error: &EvaluateError<'_>,
) -> Result<Step, GuardrailFailure> {
    let status = error.reply.status.as_u16();
    if matches!(status, 408 | 429) {
        let reason = format!("the Agent 365 endpoint returned HTTP {status}");
        throttled(
            guardrail,
            data,
            error.tool_name,
            &reason,
            Some(error.latency),
        )?;
        return Ok(Step::Done);
    }
    if (400..500).contains(&status) {
        return reject(guardrail, data, error);
    }
    if status != 200 {
        return unavailable_for_status(guardrail, data, error, status);
    }
    Ok(Step::Continue)
}

// LiteLLM's 4xx branch of _handle_evaluate_error: 401 also drops the cached OBO token
fn reject(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    error: &EvaluateError<'_>,
) -> Result<Step, GuardrailFailure> {
    let status = error.reply.status.as_u16();
    if status == 401 {
        guardrail.evict_obo_token(error.assertion);
    }
    let text = error_text(error.reply);
    let reason = format!("HTTP {status}: {text}");
    write_record(
        guardrail,
        data,
        &VerdictRecord {
            verdict: Verdict::Rejected,
            defender_status: None,
            correlation_id: None,
            latency: Some(error.latency),
            reason: Some(&reason),
        },
    )?;
    let detail = UnavailableDetail {
        error: REJECTED_ERROR.to_owned(),
        message: rejection_message(status, text),
        tool: error.tool_name.to_owned(),
    };
    Err(GuardrailFailure::HttpObject {
        status: REJECTED_STATUS,
        detail: detail.detail(),
    })
}

fn unavailable_for_status(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    error: &EvaluateError<'_>,
    status: u16,
) -> Result<Step, GuardrailFailure> {
    let reason = format!("the Agent 365 endpoint returned HTTP {status}");
    unavailable(
        guardrail,
        data,
        error.tool_name,
        &unavailable_record(&reason),
    )?;
    Ok(Step::Done)
}

fn rejection_message(status: u16, text: String) -> String {
    if status == REJECTED_STATUS {
        return text;
    }
    format!("the Agent 365 evaluation request failed with HTTP {status}")
}

// LiteLLM's _enforce_verdict
fn enforce_verdict(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    tool_name: &str,
    reply: &PostReply,
    latency: Duration,
) -> Result<Step, GuardrailFailure> {
    let Some(verdict) = decode_or_unavailable(guardrail, data, tool_name, &reply.body)? else {
        return Ok(Step::Done);
    };
    if verdict.allowed && !verdict.defender_evaluated() {
        let reason = verdict.unevaluated_reason();
        unavailable(
            guardrail,
            data,
            tool_name,
            &VerdictRecord {
                verdict: Verdict::Unavailable,
                defender_status: verdict.defender.status.as_deref(),
                correlation_id: verdict.correlation_id.as_deref(),
                latency: Some(latency),
                reason: Some(&reason),
            },
        )?;
        return Ok(Step::Done);
    }
    record_decision(guardrail, data, &verdict, latency)?;
    if verdict.allowed {
        return Ok(Step::Done);
    }
    Err(blocked_failure(tool_name, &verdict))
}

// Ok(None) means an unavailable fallback already answered the call
fn decode_or_unavailable(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    tool_name: &str,
    body: &[u8],
) -> Result<Option<EvaluateVerdict>, GuardrailFailure> {
    let verdict = match decode_evaluate_response(body) {
        Ok(verdict) => verdict,
        Err(error) => {
            let reason = error.to_string();
            unavailable(guardrail, data, tool_name, &unavailable_record(&reason))?;
            return Ok(None);
        }
    };
    Ok(Some(verdict))
}

fn unavailable_record(reason: &str) -> VerdictRecord<'_> {
    VerdictRecord {
        verdict: Verdict::Unavailable,
        defender_status: None,
        correlation_id: None,
        latency: None,
        reason: Some(reason),
    }
}

fn record_decision(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    verdict: &EvaluateVerdict,
    latency: Duration,
) -> Result<(), GuardrailFailure> {
    let recorded = if verdict.allowed {
        Verdict::Allow
    } else {
        Verdict::Block
    };
    write_record(
        guardrail,
        data,
        &VerdictRecord {
            verdict: recorded,
            defender_status: verdict.defender.status.as_deref(),
            correlation_id: verdict.correlation_id.as_deref(),
            latency: Some(latency),
            reason: None,
        },
    )
}

fn blocked_failure(tool_name: &str, verdict: &EvaluateVerdict) -> GuardrailFailure {
    let detail = BlockedDetail {
        error: BLOCKED_ERROR.to_owned(),
        message: verdict.blocked_message(tool_name),
        tool: tool_name.to_owned(),
        correlation_id: verdict.correlation_id.clone(),
    };
    GuardrailFailure::HttpObject {
        status: REJECTED_STATUS,
        detail: detail.detail(),
    }
}

fn error_text(reply: &PostReply) -> String {
    let text = std::str::from_utf8(&reply.body).unwrap_or_default();
    text.chars().take(MAX_ERROR_TEXT_CHARS).collect()
}

fn write_record(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    record: &VerdictRecord<'_>,
) -> Result<(), GuardrailFailure> {
    let written = record_verdict(guardrail, data, record);
    written.map_err(|error| {
        GuardrailFailure::Other(format!(
            "the Agent 365 guardrail record could not be written: {error}"
        ))
    })?;
    Ok(())
}
