use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::repello_content::wire::{RepelloAIAnalyzeResponse, RepelloAIViolatedPolicy};

use super::{
    BLOCKED_DETAIL, BLOCKED_STATUS, BLOCKED_VERDICT, FLAGGED_VERDICT, PASSED_VERDICT,
    POLICIES_PREFIX, UNKNOWN_POLICY,
};

// LiteLLM's _verdict_blocks: no response never blocks, and a missing or unknown verdict blocks.
#[must_use]
pub fn verdict_blocks(response: Option<&RepelloAIAnalyzeResponse>) -> bool {
    let Some(response) = response else {
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

// LiteLLM's _format_blocked_detail: only the policies that name an action or a score are listed.
#[must_use]
pub fn format_blocked_detail(response: &RepelloAIAnalyzeResponse) -> String {
    let Some(policies) = response
        .policies_violated
        .as_deref()
        .filter(|policies| !policies.is_empty())
    else {
        return BLOCKED_DETAIL.to_owned();
    };
    let formatted: Vec<String> = policies.iter().map(format_policy).collect();
    format!("{POLICIES_PREFIX}{}.", formatted.join("; "))
}

fn format_policy(policy: &RepelloAIViolatedPolicy) -> String {
    let name = policy.policy_name.as_deref().unwrap_or(UNKNOWN_POLICY);
    let mut parts: Vec<String> = Vec::new();
    if let Some(action) = policy.action_taken.as_deref() {
        parts.push(format!("action: {action}"));
    }
    if let Some(score) = policy
        .details
        .as_ref()
        .and_then(|details| details.get("score"))
    {
        parts.push(format!("score: {}", rendered_score(score)));
    }
    if parts.is_empty() {
        return name.to_owned();
    }
    format!("{name} ({})", parts.join(", "))
}

fn rendered_score(score: &GuardrailValue) -> String {
    if let GuardrailValue::Text(text) = score {
        return text.clone();
    }
    serde_json::to_string(score).unwrap_or_else(|error| {
        tracing::warn!("RepelloAI Argus policy score could not be rendered: {error}");
        String::new()
    })
}

// LiteLLM's _raise_if_blocked: a block raises the 400 with the formatted detail, a flag is only logged.
pub fn raise_if_blocked(
    response: Option<&RepelloAIAnalyzeResponse>,
) -> Result<(), GuardrailFailure> {
    let Some(response) = response else {
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
    tracing::warn!(
        "RepelloAI Argus flagged content (allowed): {}",
        policies_text(response)
    );
}

fn policies_text(response: &RepelloAIAnalyzeResponse) -> String {
    let Some(policies) = response.policies_violated.as_ref() else {
        return "None".to_owned();
    };
    serde_json::to_string(policies).unwrap_or_else(|error| {
        tracing::warn!("RepelloAI Argus flagged policies could not be logged: {error}");
        String::new()
    })
}
