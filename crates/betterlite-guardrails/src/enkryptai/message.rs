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
        "pii" => {
            let pii = response
                .details
                .pii
                .as_ref()
                .and_then(|value| value.pii.clone())
                .unwrap_or_default();
            Ok(format!("pii detected: {}", serde_json::to_string(&pii)?))
        }
        "toxicity" => Ok(format!(
            "toxicity scores above {TOXICITY_THRESHOLD}: {}",
            toxicity_names(response.details.toxicity.as_ref()).join(", ")
        )),
        _ => describe_detector(attack, processed, response),
    }
}

fn policy(response: &DetectionResponse) -> String {
    let detail = response.details.policy_violation.as_ref();
    let policy = detail
        .and_then(|value| value.violating_policy.as_deref())
        .unwrap_or(NOT_AVAILABLE);
    let explanation = detail
        .and_then(|value| value.explanation.as_deref())
        .unwrap_or(NOT_AVAILABLE);
    format!("policy \"{policy}\", explanation \"{explanation}\"")
}

fn describe_detector(
    attack: &str,
    processed: &Processed,
    response: &DetectionResponse,
) -> Result<String, serde_json::Error> {
    match attack {
        "keyword_detected" => {
            let keywords = response
                .details
                .keyword_detected
                .as_ref()
                .and_then(|value| value.detected_keywords.clone())
                .unwrap_or_default();
            Ok(format!(
                "detected keywords: {}",
                serde_json::to_string(&keywords)?
            ))
        }
        "bias" => {
            let detected = response
                .details
                .bias
                .as_ref()
                .and_then(|value| value.bias_detected)
                .unwrap_or(false);
            Ok(format!("bias detected: {detected}"))
        }
        other => {
            let detail = processed
                .details
                .get(other)
                .cloned()
                .unwrap_or_else(|| GuardrailValue::Object(BTreeMap::new()));
            Ok(format!("details: {}", serde_json::to_string(&detail)?))
        }
    }
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
