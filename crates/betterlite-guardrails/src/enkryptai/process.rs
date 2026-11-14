use crate::guardrail_records::record::GuardrailStatus;
use crate::primitives::json_value::JsonValue;

use super::wire::DetectionResponse;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Processed {
    pub attacks: Vec<String>,
}

impl Processed {
    #[must_use]
    pub const fn status(&self) -> GuardrailStatus {
        if self.attacks.is_empty() {
            GuardrailStatus::Success
        } else {
            GuardrailStatus::GuardrailIntervened
        }
    }
}

#[must_use]
pub fn process(response: &DetectionResponse) -> Processed {
    let mut attacks = Vec::new();
    for (key, value) in &response.summary.0 {
        let detected = if key == "toxicity" {
            matches!(value, JsonValue::Array(types) if !types.is_empty())
        } else {
            is_detected(value)
        };
        if detected {
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
