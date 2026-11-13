use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::process::Processed;
use super::wire::{DetectionResponse, ToxicityDetail};

const NOT_AVAILABLE: &str = "N/A";
const TOXICITY_THRESHOLD: f64 = 0.5;

pub(crate) fn detected(
    processed: &Processed,
    response: &DetectionResponse,
) -> Result<String, serde_json::Error> {
    let mut message = format!(
        "EnkryptAI guardrail detected {} violation(s):",
        processed.attacks.len()
    );
    for attack in &processed.attacks {
        message.push_str("\n- ");
        message.push_str(&attack.to_ascii_uppercase());
        message.push_str(": ");
        message.push_str(&describe(attack, processed, response)?);
    }
    Ok(message)
}

fn describe(
    attack: &str,
    processed: &Processed,
    response: &DetectionResponse,
) -> Result<String, serde_json::Error> {
    match attack {
        "policy_violation" => Ok(policy(response)),
        "pii" => Ok(format!(
            "pii detected: {}",
            serde_json::to_string(&pii_details(response))?
        )),
        "toxicity" => Ok(format!(
            "toxicity scores above {TOXICITY_THRESHOLD}: {}",
            toxicity_names(response.details.toxicity.as_ref()).join(", ")
        )),
        "keyword_detected" => Ok(format!(
            "detected keywords: {}",
            serde_json::to_string(&keywords(response))?
        )),
        "bias" => Ok(format!("bias detected: {}", bias_detected(response))),
        other => unknown(other, processed),
    }
}

fn pii_details(response: &DetectionResponse) -> BTreeMap<String, GuardrailValue> {
    match &response.details.pii {
        Some(detail) => detail.pii.clone().unwrap_or_default(),
        None => BTreeMap::new(),
    }
}

fn keywords(response: &DetectionResponse) -> Vec<String> {
    match &response.details.keyword_detected {
        Some(detail) => detail.detected_keywords.clone().unwrap_or_default(),
        None => Vec::new(),
    }
}

fn bias_detected(response: &DetectionResponse) -> bool {
    match &response.details.bias {
        Some(detail) => detail.bias_detected.unwrap_or(false),
        None => false,
    }
}

fn unknown(attack: &str, processed: &Processed) -> Result<String, serde_json::Error> {
    let detail = processed
        .details
        .get(attack)
        .cloned()
        .unwrap_or_else(|| GuardrailValue::Object(BTreeMap::new()));
    Ok(format!("details: {}", serde_json::to_string(&detail)?))
}

fn toxicity_names(detail: Option<&ToxicityDetail>) -> Vec<&'static str> {
    let Some(detail) = detail else {
        return Vec::new();
    };
    let scores = [
        ("toxic", detail.toxic),
        ("severe_toxic", detail.severe_toxic),
        ("obscene", detail.obscene),
        ("threat", detail.threat),
        ("insult", detail.insult),
        ("identity_hate", detail.identity_hate),
    ];
    scores
        .into_iter()
        .filter_map(|(name, score)| {
            score
                .filter(|value| *value > TOXICITY_THRESHOLD)
                .map(|_| name)
        })
        .collect()
}
