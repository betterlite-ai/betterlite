use std::time::Duration;

use betterlite_config::sections::guardrail_params::base::UnreachableFallback;

use super::Step;
use super::record::{VerdictRecord, record_verdict};
use super::refusals::{Refusal, UNAVAILABLE_ERROR, UNAVAILABLE_STATUS, refusal_failure, refuse};
use crate::agent_365::enforcement_types::{
    BlockedDetail, EvaluateVerdict, UnavailableDetail, Verdict, decode_evaluate_response,
};
use crate::agent_365_tokens::Agent365Guardrail;
use crate::agent_365_tokens::poster::PostReply;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_metadata::RequestData;

pub(super) const REJECTED_ERROR: &str = "Agent 365 rejected the tool evaluation request";
pub(super) const BLOCKED_ERROR: &str = "Blocked by Microsoft Defender";
const REJECTED_STATUS: u16 = 400;
const MAX_ERROR_TEXT_CHARS: usize = 512;

// The arguments of _handle_evaluate_error; Continue means the verdict still has to be enforced
pub(super) struct EvaluateError<'reply> {
    pub tool_name: &'reply str,
    pub assertion: &'reply str,
    pub reply: &'reply PostReply,
    pub latency: Duration,
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
    record_verdict(guardrail, data, record)?;
    let message = format!(
        "Tool call '{tool_name}' was blocked because {reason} and unreachable_fallback is 'fail_closed'."
    );
    Err(refusal_failure(
        UNAVAILABLE_STATUS,
        UNAVAILABLE_ERROR,
        message,
        tool_name,
    ))
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
    record_verdict(guardrail, data, &record.relabelled(Verdict::Unscanned))
}

// Runs _handle_evaluate_error and, when it lets the call through, _enforce_verdict
pub(super) fn enforce_reply(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    error: &EvaluateError<'_>,
) -> Result<(), GuardrailFailure> {
    if matches!(
        handle_evaluate_error(guardrail, data, error)?,
        Step::Continue
    ) {
        enforce_verdict(guardrail, data, error.tool_name, error.reply, error.latency)
    } else {
        Ok(())
    }
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
        refuse(
            guardrail,
            data,
            &Refusal::Throttled {
                tool_name: error.tool_name,
                reason: &reason,
                latency: Some(error.latency),
            },
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
    let body = std::str::from_utf8(&error.reply.body).unwrap_or_default();
    let text: String = body.chars().take(MAX_ERROR_TEXT_CHARS).collect();
    let reason = format!("HTTP {status}: {text}");
    record_verdict(
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
    let message = if status == REJECTED_STATUS {
        text
    } else {
        format!("the Agent 365 evaluation request failed with HTTP {status}")
    };
    let detail = UnavailableDetail {
        error: REJECTED_ERROR.to_owned(),
        message,
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

// LiteLLM's _enforce_verdict
fn enforce_verdict(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    tool_name: &str,
    reply: &PostReply,
    latency: Duration,
) -> Result<(), GuardrailFailure> {
    let verdict = match decode_evaluate_response(&reply.body) {
        Ok(verdict) => verdict,
        // an unavailable fallback already answered the call
        Err(error) => {
            let reason = error.to_string();
            unavailable(guardrail, data, tool_name, &unavailable_record(&reason))?;
            return Ok(());
        }
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
        return Ok(());
    }
    record_decision(guardrail, data, &verdict, latency)?;
    if verdict.allowed {
        return Ok(());
    }
    Err(blocked_failure(tool_name, &verdict))
}

pub(super) fn unavailable_record(reason: &str) -> VerdictRecord<'_> {
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
    record_verdict(
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
