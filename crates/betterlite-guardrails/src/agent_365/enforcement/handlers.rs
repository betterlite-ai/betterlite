use std::str;
use std::time::Duration;

use betterlite_config::sections::guardrail_params::base::UnreachableFallback;

use super::Step;
use super::record::{VerdictRecord, record_verdict};
use super::refusals::{Refusal, UNAVAILABLE_ERROR, UNAVAILABLE_STATUS, refusal_failure, refuse};
use crate::agent_365::enforcement_types::{
    BlockedDetail, EvaluateVerdict, UnavailableDetail, Verdict, decode_evaluate_response,
};
use crate::agent_365_tokens::Agent365Guardrail;
use crate::agent_365_tokens::DEFENDER_STATUS_EVALUATED;
use crate::agent_365_tokens::poster::PostReply;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_metadata::RequestData;

pub(super) const REJECTED_ERROR: &str = "Agent 365 rejected the tool evaluation request";
pub(super) const BLOCKED_ERROR: &str = "Blocked by Microsoft Defender";
const REJECTED_STATUS: u16 = 400;
const MAX_ERROR_TEXT_CHARS: usize = 512;

// The evaluation endpoint's reply with what _handle_evaluate_error reads beside it
pub(super) struct EvaluationReply<'reply> {
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
        allow_unscanned(guardrail, data, tool_name, record);
        return Ok(());
    }
    record_verdict(guardrail, data, record);
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
) {
    let reason = record.reason.unwrap_or_default();
    let name = &guardrail.settings.guardrail_name;
    tracing::error!(
        "Agent 365 guardrail ({name}): {reason}; unreachable_fallback='fail_open', allowing tool call '{tool_name}' unscanned"
    );
    record_verdict(
        guardrail,
        data,
        &VerdictRecord {
            verdict: Verdict::Unscanned,
            ..*record
        },
    );
}

// Runs _handle_evaluate_error and, when it lets the call through, _enforce_verdict
pub(super) fn enforce_reply(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    evaluation: &EvaluationReply<'_>,
) -> Result<(), GuardrailFailure> {
    if matches!(
        handle_evaluate_error(guardrail, data, evaluation)?,
        Step::Continue
    ) {
        enforce_verdict(
            guardrail,
            data,
            evaluation.tool_name,
            evaluation.reply,
            evaluation.latency,
        )
    } else {
        Ok(())
    }
}

// LiteLLM's _handle_evaluate_error; 408 and 429 are checked before the rest of the 4xx range
fn handle_evaluate_error(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    evaluation: &EvaluationReply<'_>,
) -> Result<Step, GuardrailFailure> {
    let status = evaluation.reply.status.as_u16();
    if matches!(status, 408 | 429) {
        let reason = format!("the Agent 365 endpoint returned HTTP {status}");
        refuse(
            guardrail,
            data,
            &Refusal::Throttled {
                tool_name: evaluation.tool_name,
                reason: &reason,
                latency: Some(evaluation.latency),
            },
        )?;
        return Ok(Step::Done);
    }
    if (400..500).contains(&status) {
        return reject(guardrail, data, evaluation);
    }
    if status != 200 {
        return unavailable_for_status(guardrail, data, evaluation, status);
    }
    Ok(Step::Continue)
}

// LiteLLM's 4xx branch of _handle_evaluate_error: 401 also drops the cached OBO token
fn reject(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    evaluation: &EvaluationReply<'_>,
) -> Result<Step, GuardrailFailure> {
    let status = evaluation.reply.status.as_u16();
    if status == 401 {
        guardrail.evict_obo_token(evaluation.assertion);
    }
    let body = str::from_utf8(&evaluation.reply.body).unwrap_or_default();
    let text: String = body.chars().take(MAX_ERROR_TEXT_CHARS).collect();
    let reason = format!("HTTP {status}: {text}");
    record_verdict(
        guardrail,
        data,
        &VerdictRecord {
            verdict: Verdict::Rejected,
            defender_status: None,
            correlation_id: None,
            latency: Some(evaluation.latency),
            reason: Some(&reason),
        },
    );
    let message = if status == REJECTED_STATUS {
        text
    } else {
        format!("the Agent 365 evaluation request failed with HTTP {status}")
    };
    let detail = UnavailableDetail {
        error: REJECTED_ERROR.to_owned(),
        message,
        tool: evaluation.tool_name.to_owned(),
    };
    Err(GuardrailFailure::HttpObject {
        status: REJECTED_STATUS,
        detail: detail.detail(),
    })
}

fn unavailable_for_status(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    evaluation: &EvaluationReply<'_>,
    status: u16,
) -> Result<Step, GuardrailFailure> {
    let reason = format!("the Agent 365 endpoint returned HTTP {status}");
    unavailable(
        guardrail,
        data,
        evaluation.tool_name,
        &VerdictRecord {
            verdict: Verdict::Unavailable,
            defender_status: None,
            correlation_id: None,
            latency: None,
            reason: Some(&reason),
        },
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
            unavailable(
                guardrail,
                data,
                tool_name,
                &VerdictRecord {
                    verdict: Verdict::Unavailable,
                    defender_status: None,
                    correlation_id: None,
                    latency: None,
                    reason: Some(&reason),
                },
            )?;
            return Ok(());
        }
    };
    if verdict.allowed && verdict.defender.status.as_deref() != Some(DEFENDER_STATUS_EVALUATED) {
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
    record_decision(guardrail, data, &verdict, latency);
    if verdict.allowed {
        return Ok(());
    }
    Err(blocked_failure(tool_name, &verdict))
}

fn record_decision(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    verdict: &EvaluateVerdict,
    latency: Duration,
) {
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
    );
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
