use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::repello_content::wire::{RepelloAIAnalyzeResponse, RepelloAIViolatedPolicy};

use super::{
    BLOCKED_DETAIL, BLOCKED_STATUS, BLOCKED_VERDICT, FLAGGED_VERDICT, PASSED_VERDICT,
    POLICIES_PREFIX, UNKNOWN_POLICY,
};

// LiteLLM's _verdict_blocks: no response never blocks, and a missing or unknown verdict blocks.
#[must_use]
pub fn verdict_blocks(scanned: Option<&RepelloAIAnalyzeResponse>) -> bool {
    let Some(response) = scanned else {
        return false;
    };
    match response.verdict.as_deref() {
        Some(BLOCKED_VERDICT) => true,
        Some(PASSED_VERDICT | FLAGGED_VERDICT) => false,
        other => {
            tracing::warn!(
                "RepelloAI Argus returned an unrecognized verdict ({}) - blocking.",
                other.unwrap_or("None")
            );
            true
        }
    }
}

// LiteLLM's _format_blocked_detail: every policy is listed, and its action and score appear only when present.
#[must_use]
pub fn format_blocked_detail(response: &RepelloAIAnalyzeResponse) -> String {
    let policies = response.policies_violated.as_deref().unwrap_or_default();
    if policies.is_empty() {
        return BLOCKED_DETAIL.to_owned();
    }
    let formatted: Vec<String> = policies.iter().map(format_policy).collect();
    format!("{POLICIES_PREFIX}{}.", formatted.join("; "))
}

fn format_policy(policy: &RepelloAIViolatedPolicy) -> String {
    let name = policy.policy_name.as_deref().unwrap_or(UNKNOWN_POLICY);
    let action = policy
        .action_taken
        .as_deref()
        .map(|action| format!("action: {action}"));
    let score = policy
        .details
        .as_ref()
        .and_then(|details| details.get("score"))
        .map(|score| {
            if let GuardrailValue::Text(text) = score {
                format!("score: {text}")
            } else {
                format!(
                    "score: {}",
                    serde_json::to_string(score).unwrap_or_default()
                )
            }
        });
    let parts: Vec<String> = action.into_iter().chain(score).collect();
    if parts.is_empty() {
        return name.to_owned();
    }
    format!("{name} ({})", parts.join(", "))
}

// LiteLLM's _raise_if_blocked: a block raises the 400 with the formatted detail, a flag is only logged.
pub fn raise_if_blocked(
    scanned: Option<&RepelloAIAnalyzeResponse>,
) -> Result<(), GuardrailFailure> {
    let Some(response) = scanned else {
        return Ok(());
    };
    if verdict_blocks(Some(response)) {
        return Err(GuardrailFailure::Http {
            status: BLOCKED_STATUS,
            detail: format_blocked_detail(response),
        });
    }
    log_flagged_verdict(response);
    Ok(())
}

pub fn log_flagged_verdict(response: &RepelloAIAnalyzeResponse) {
    if response.verdict.as_deref() != Some(FLAGGED_VERDICT) {
        return;
    }
    let Some(policies) = response.policies_violated.as_ref() else {
        tracing::warn!("RepelloAI Argus flagged content (allowed): None");
        return;
    };
    match serde_json::to_string(policies) {
        Ok(text) => tracing::warn!("RepelloAI Argus flagged content (allowed): {text}"),
        Err(error) => {
            tracing::warn!("RepelloAI Argus flagged policies could not be logged: {error}");
        }
    }
}
