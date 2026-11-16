use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};

use crate::straiker::v3::blocked_by::blocking_controls;
use crate::straiker::v3::decision_mapping::decision;
use crate::straiker::v3::{BLOCK_DECISIONS, UNDECIDED_DECISIONS};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum StraikerWebhookAction {
    #[serde(rename = "NONE")]
    NoAction,
    #[serde(rename = "BLOCKED")]
    Blocked,
    #[serde(rename = "GUARDRAIL_INTERVENED")]
    GuardrailIntervened,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StraikerWebhookResponse {
    pub action: StraikerWebhookAction,
    pub blocked_reason: Option<String>,
    pub blocked_by: Vec<String>,
    #[serde(rename = "turnId")]
    pub turn_id: Option<String>,
}

// A detect-mode control fires into `controls` without changing the decision, so it reads as no action. `blocked_by` is the block-mode subset and is honoured even if a build answers it without flipping the decision. None when Straiker stated no verdict, or said `ask`, which a gateway has no one to put to.
#[must_use]
pub fn response(body: &Object) -> Option<StraikerWebhookResponse> {
    let (stated, verdict) = decision(body);
    let controls = blocking_controls(verdict);
    let undecided = stated
        .as_deref()
        .is_none_or(|text| UNDECIDED_DECISIONS.contains(&text));
    if undecided && controls.is_empty() {
        return None;
    }
    let blocked = !controls.is_empty()
        || stated
            .as_deref()
            .is_some_and(|text| BLOCK_DECISIONS.contains(&text));
    let blocked_reason = blocked.then(|| {
        [
            verdict.get("block_message"),
            verdict.get("deny_reason"),
            body.get("stopReason"),
        ]
        .into_iter()
        .flatten()
        .filter_map(Json::as_str)
        .map(str::trim)
        .find(|text| !text.is_empty())
        .map_or_else(
            || {
                let joined = controls.join(", ");
                let named = if joined.is_empty() { "policy" } else { &joined };
                format!("Straiker blocked this turn: {named}")
            },
            str::to_owned,
        )
    });
    Some(StraikerWebhookResponse {
        action: if blocked {
            StraikerWebhookAction::Blocked
        } else {
            StraikerWebhookAction::NoAction
        },
        blocked_reason,
        blocked_by: controls,
        turn_id: turn_id(verdict).or_else(|| turn_id(body)),
    })
}

fn turn_id(fields: &Object) -> Option<String> {
    fields
        .get("turn_id")
        .and_then(Json::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

mod tests;
