use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;

// A null decision value still selects its envelope, so presence cannot use Option.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Decision {
    action: Presence,
    allowed: Presence,
    blocked: Presence,
    safe: Presence,
    is_safe: Presence,
    #[serde(rename = "decision")]
    outcome: Presence,
    verdict: Presence,
    status: Presence,
    score: Presence,
    risk_score: Presence,
    confidence: Presence,
    categories: Presence,
    classifications: Presence,
    violations: Presence,
    threats: Presence,
    policies: Presence,
    reason: Presence,
    rules: Presence,
    sanitized_text: Presence,
    #[serde(rename = "sanitizedText")]
    sanitized_text_camel: Presence,
    sanitized_payload: Presence,
}

#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum Presence {
    Present(IgnoredAny),
    #[default]
    Missing,
}

impl Presence {
    const fn is_present(&self) -> bool {
        match self {
            Self::Present(_) => true,
            Self::Missing => false,
        }
    }
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
    decision.action.is_present()
        || decision.allowed.is_present()
        || decision.blocked.is_present()
        || decision.safe.is_present()
        || decision.is_safe.is_present()
        || decision.outcome.is_present()
        || decision.verdict.is_present()
        || decision.status.is_present()
        || decision.score.is_present()
        || decision.risk_score.is_present()
        || decision.confidence.is_present()
        || decision.categories.is_present()
        || decision.classifications.is_present()
        || decision.violations.is_present()
        || decision.threats.is_present()
        || decision.policies.is_present()
        || decision.reason.is_present()
        || decision.rules.is_present()
        || decision.sanitized_text.is_present()
        || decision.sanitized_text_camel.is_present()
        || decision.sanitized_payload.is_present()
}

mod tests;
