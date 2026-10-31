use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;

// LiteLLM's `_DECISION_FIELDS` tuple, as a closed struct whose fields only record presence.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Decision {
    action: Option<IgnoredAny>,
    allowed: Option<IgnoredAny>,
    blocked: Option<IgnoredAny>,
    safe: Option<IgnoredAny>,
    is_safe: Option<IgnoredAny>,
    #[serde(rename = "decision")]
    outcome: Option<IgnoredAny>,
    verdict: Option<IgnoredAny>,
    status: Option<IgnoredAny>,
    score: Option<IgnoredAny>,
    risk_score: Option<IgnoredAny>,
    confidence: Option<IgnoredAny>,
    categories: Option<IgnoredAny>,
    classifications: Option<IgnoredAny>,
    violations: Option<IgnoredAny>,
    threats: Option<IgnoredAny>,
    policies: Option<IgnoredAny>,
    reason: Option<IgnoredAny>,
    rules: Option<IgnoredAny>,
    sanitized_text: Option<IgnoredAny>,
    #[serde(rename = "sanitizedText")]
    sanitized_text_camel: Option<IgnoredAny>,
    sanitized_payload: Option<IgnoredAny>,
}

#[must_use]
pub fn has_decision_fields(payload: &Json) -> bool {
    let Json::Object(fields) = payload else {
        return false;
    };
    object_has_decision_fields(fields)
}

fn object_has_decision_fields(fields: &Fields) -> bool {
    let Ok(text) = serde_json::to_string(fields) else {
        return false;
    };
    let Ok(decision) = serde_json::from_str::<Decision>(&text) else {
        return false;
    };
    decision.action.is_some()
        || decision.allowed.is_some()
        || decision.blocked.is_some()
        || decision.safe.is_some()
        || decision.is_safe.is_some()
        || decision.outcome.is_some()
        || decision.verdict.is_some()
        || decision.status.is_some()
        || decision.score.is_some()
        || decision.risk_score.is_some()
        || decision.confidence.is_some()
        || decision.categories.is_some()
        || decision.classifications.is_some()
        || decision.violations.is_some()
        || decision.threats.is_some()
        || decision.policies.is_some()
        || decision.reason.is_some()
        || decision.rules.is_some()
        || decision.sanitized_text.is_some()
        || decision.sanitized_text_camel.is_some()
        || decision.sanitized_payload.is_some()
}

mod tests;
