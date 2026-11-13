use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::json_header::encode_json_for_header;

const TRUNCATED_VALUE: &str = "[truncated]";
const TRUNCATION_SUFFIX: &str = "...[truncated]";

#[derive(Debug, Clone, PartialEq)]
pub struct TruncatedEvidence {
    pub evidence: GuardrailValue,
    pub encoded: String,
    pub was_truncated: bool,
}

fn encoded_array(entries: &[GuardrailValue]) -> String {
    encode_json_for_header(&GuardrailValue::Array(entries.to_vec()))
}

// The text LiteLLM shortens: the string in the entry's evidence field, or none when it holds no string
fn evidence_text(fields: &BTreeMap<String, GuardrailValue>) -> Option<String> {
    match fields.get("evidence") {
        None | Some(GuardrailValue::Null) => Some(String::new()),
        Some(GuardrailValue::Text(text)) => Some(text.clone()),
        Some(
            GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Array(_)
            | GuardrailValue::Object(_),
        ) => None,
    }
}

fn shorten_entry(entry: &mut GuardrailValue, encoded: &mut String, max_bytes: usize) {
    let GuardrailValue::Object(fields) = entry else {
        return;
    };
    let Some(mut text) = evidence_text(fields) else {
        return;
    };
    if text.is_empty() {
        return;
    }
    let mut step = text.chars().count().checked_div(2).unwrap_or(0).max(1);
    while encoded.len() > max_bytes && !text.is_empty() {
        let characters = text.chars().count();
        text = if characters > step {
            text.chars().take(characters.saturating_sub(step)).collect()
        } else {
            text.chars().take(characters.saturating_sub(1)).collect()
        };
        step = step.checked_div(2).unwrap_or(0).max(1);
        let replaced = if text.is_empty() {
            TRUNCATED_VALUE.to_owned()
        } else {
            format!("{text}{TRUNCATION_SUFFIX}")
        };
        fields.insert("evidence".to_owned(), GuardrailValue::Text(replaced));
        fields.insert(
            "evidence_truncated".to_owned(),
            GuardrailValue::Boolean(true),
        );
        *encoded = encode_json_for_header(entry);
    }
}

#[must_use]
pub fn truncate_evidence_payload(evidence: &GuardrailValue, max_bytes: usize) -> TruncatedEvidence {
    let GuardrailValue::Array(entries) = evidence else {
        let encoded = encode_json_for_header(evidence);
        if encoded.len() <= max_bytes {
            return TruncatedEvidence {
                evidence: evidence.clone(),
                encoded,
                was_truncated: false,
            };
        }
        let truncated = GuardrailValue::Text(TRUNCATED_VALUE.to_owned());
        return TruncatedEvidence {
            encoded: encode_json_for_header(&truncated),
            evidence: truncated,
            was_truncated: true,
        };
    };
    let mut kept: Vec<GuardrailValue> = Vec::new();
    let mut encoded = encoded_array(&kept);
    let mut was_truncated = false;
    for entry in entries {
        kept.push(entry.clone());
        let mut current = encoded_array(&kept);
        if current.len() <= max_bytes {
            encoded = current;
            continue;
        }
        was_truncated = true;
        if let Some(last) = kept.last_mut() {
            shorten_entry(last, &mut current, max_bytes);
        }
        if current.len() <= max_bytes {
            encoded = current;
            continue;
        }
        kept.pop();
        encoded = encoded_array(&kept);
    }
    TruncatedEvidence {
        evidence: GuardrailValue::Array(kept),
        encoded,
        was_truncated,
    }
}
