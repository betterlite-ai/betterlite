use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_records::record::GuardrailStatus;

use super::wire::{DetectionResponse, Summary};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Processed {
    pub attacks: Vec<String>,
    pub details: BTreeMap<String, GuardrailValue>,
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
pub fn process(response: &DetectionResponse, raw: &GuardrailValue) -> Processed {
    let attacks = attacks(&response.summary);
    let details = attacks
        .iter()
        .map(|attack| (attack.clone(), raw_detail(raw, attack)))
        .collect();
    Processed { attacks, details }
}

fn attacks(summary: &Summary) -> Vec<String> {
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
    attacks
}

fn is_detected(value: &GuardrailValue) -> bool {
    match value {
        GuardrailValue::SignedInteger(1) | GuardrailValue::UnsignedInteger(1) => true,
        GuardrailValue::Decimal(number) => *number == 1.0,
        _ => false,
    }
}

fn raw_detail(raw: &GuardrailValue, key: &str) -> GuardrailValue {
    let GuardrailValue::Object(fields) = raw else {
        return GuardrailValue::Object(BTreeMap::new());
    };
    let Some(GuardrailValue::Object(details)) = fields.get("details") else {
        return GuardrailValue::Object(BTreeMap::new());
    };
    details
        .get(key)
        .cloned()
        .unwrap_or_else(|| GuardrailValue::Object(BTreeMap::new()))
}
