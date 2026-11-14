use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;

use crate::guardrail_records::record::GuardrailStatus;

use super::error::EnkryptAiError;
use super::wire::{DetectionResponse, Summary};

const EMPTY: GuardrailValue = GuardrailValue::Object(BTreeMap::new());

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

pub fn process(response: &DetectionResponse) -> Result<Processed, EnkryptAiError> {
    let attacks = attacks(&response.summary);
    let mut details = BTreeMap::new();
    for attack in &attacks {
        details.insert(attack.clone(), detail(response, attack)?);
    }
    Ok(Processed { attacks, details })
}

fn detail(response: &DetectionResponse, attack: &str) -> Result<GuardrailValue, EnkryptAiError> {
    let details = &response.details;
    match attack {
        "policy_violation" => detail_value(details.policy_violation.as_ref()),
        "pii" => detail_value(details.pii.as_ref()),
        "toxicity" => detail_value(details.toxicity.as_ref()),
        "keyword_detected" => detail_value(details.keyword_detected.as_ref()),
        "bias" => detail_value(details.bias.as_ref()),
        "prompt_injection" => detail_value(details.prompt_injection.as_ref()),
        "jailbreak" => detail_value(details.jailbreak.as_ref()),
        other => Ok(details.other.get(other).cloned().unwrap_or(EMPTY)),
    }
}

fn detail_value<T>(value: Option<&T>) -> Result<GuardrailValue, EnkryptAiError>
where
    T: Serialize,
{
    let Some(value) = value else {
        return Ok(EMPTY);
    };
    serde_json::to_value(value)
        .map_err(|source| EnkryptAiError::Serialize { source })
        .and_then(|value| {
            serde_json::from_value(value).map_err(|source| EnkryptAiError::Serialize { source })
        })
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
