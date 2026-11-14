use serde::Serialize;

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
        message.push_str(&describe(attack, response)?);
    }
    Ok(message)
}

fn describe(attack: &str, response: &DetectionResponse) -> Result<String, serde_json::Error> {
    let details = &response.details;
    match attack {
        "policy_violation" => Ok(policy(response)),
        "pii" => Ok(format!(
            "pii detected: {}",
            match &details.pii {
                Some(detail) => serde_json::to_string(&detail.pii)?,
                None => "{}".to_owned(),
            }
        )),
        "toxicity" => Ok(format!(
            "toxicity scores above {TOXICITY_THRESHOLD}: {}",
            toxicity_names(details.toxicity.as_ref()).join(", ")
        )),
        "keyword_detected" => Ok(format!(
            "detected keywords: {}",
            match &details.keyword_detected {
                Some(detail) => serde_json::to_string(&detail.detected_keywords)?,
                None => "[]".to_owned(),
            }
        )),
        "bias" => Ok(format!(
            "bias detected: {}",
            details
                .bias
                .as_ref()
                .is_some_and(|detail| detail.bias_detected)
        )),
        "prompt_injection" => unknown(details.prompt_injection.as_ref()),
        "jailbreak" => unknown(details.jailbreak.as_ref()),
        other => unknown(details.other.get(other)),
    }
}

fn policy(response: &DetectionResponse) -> String {
    let detail = response.details.policy_violation.as_ref();
    let violating_policy = detail
        .and_then(|detail| detail.violating_policy.as_deref())
        .unwrap_or(NOT_AVAILABLE);
    let explanation = detail
        .and_then(|detail| detail.explanation.as_deref())
        .unwrap_or(NOT_AVAILABLE);
    format!("policy \"{violating_policy}\", explanation \"{explanation}\"")
}

fn unknown<T>(detail: Option<&T>) -> Result<String, serde_json::Error>
where
    T: Serialize,
{
    match detail {
        Some(detail) => Ok(format!("details: {}", serde_json::to_string(detail)?)),
        None => Ok("details: {}".to_owned()),
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
