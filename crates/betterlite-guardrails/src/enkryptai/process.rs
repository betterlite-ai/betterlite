use crate::guardrail_records::record::GuardrailStatus;
use crate::primitives::json_value::JsonValue;

use super::wire::DetectionResponse;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Processed {
    pub attacks: Vec<String>,
}

impl Processed {
    #[must_use]
    pub fn status(&self) -> GuardrailStatus {
        if self.attacks.is_empty() {
            GuardrailStatus::Success
        } else {
            GuardrailStatus::GuardrailIntervened
        }
    }
}

#[must_use]
pub fn process(response: &DetectionResponse) -> Processed {
    let summary = &response.summary;
    let mut attacks = Vec::new();
    if summary
        .toxicity
        .as_ref()
        .is_some_and(|types| !types.is_empty())
    {
        attacks.push("toxicity".to_owned());
    }
    let integer_checks = [
        ("policy_violation", summary.policy_violation),
        ("pii", summary.pii),
        ("keyword_detected", summary.keyword_detected),
        ("bias", summary.bias),
        ("prompt_injection", summary.prompt_injection),
        ("jailbreak", summary.jailbreak),
    ];
    for (key, value) in integer_checks {
        if value == Some(1) {
            attacks.push(key.to_owned());
        }
    }
    for (key, value) in &summary.other {
        if is_detected(value) {
            attacks.push(key.clone());
        }
    }
    Processed { attacks }
}

fn is_detected(value: &JsonValue) -> bool {
    match value {
        JsonValue::Number(number) => number.as_f64() == Some(1.0),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::String(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => false,
    }
}
