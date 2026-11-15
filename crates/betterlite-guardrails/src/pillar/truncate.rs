use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use percent_encoding::utf8_percent_encode;

use super::json_header::{QUOTE_SAFE_NONE, json_text};

const TRUNCATED_VALUE: &str = "[truncated]";
const TRUNCATION_SUFFIX: &str = "...[truncated]";

#[derive(Debug, Clone, PartialEq)]
pub struct TruncatedEvidence {
    pub evidence: GuardrailValue,
    pub encoded: String,
    pub was_truncated: bool,
}

// the encoded form of the whole kept list, which is what LiteLLM measures against max_bytes
fn encoded_array(entries: &[GuardrailValue]) -> String {
    utf8_percent_encode(
        &json_text(&GuardrailValue::Array(entries.to_vec())),
        &QUOTE_SAFE_NONE,
    )
    .to_string()
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

// LiteLLM halves the evidence text of the last entry and re-encodes the whole list after every step
fn shorten_entry(kept: &mut [GuardrailValue], encoded: &mut String, max_bytes: usize) {
    let Some(GuardrailValue::Object(fields)) = kept.last() else {
        return;
    };
    let Some(mut text) = evidence_text(fields) else {
        return;
    };
    let mut step = text.chars().count().checked_div(2).unwrap_or(0).max(1);
    while encoded.len() > max_bytes && !text.is_empty() {
        let characters = text.chars().count();
        // LiteLLM drops `step` characters while the text is longer than the step, else its last one;
        // the halving step never outgrows the text until one character is left, where both drop it
        let dropped = step.min(characters);
        text = text.chars().take(characters - dropped).collect();
        step = step.checked_div(2).unwrap_or(0).max(1);
        let replaced = if text.is_empty() {
            TRUNCATED_VALUE.to_owned()
        } else {
            format!("{text}{TRUNCATION_SUFFIX}")
        };
        let Some(GuardrailValue::Object(entry_fields)) = kept.last_mut() else {
            return;
        };
        entry_fields.insert("evidence".to_owned(), GuardrailValue::Text(replaced));
        entry_fields.insert(
            "evidence_truncated".to_owned(),
            GuardrailValue::Boolean(true),
        );
        *encoded = encoded_array(kept);
    }
}

// A single evidence value is kept whole or replaced by the marker, with no shrink step
fn truncate_value(evidence: &GuardrailValue, max_bytes: usize) -> TruncatedEvidence {
    let fits = utf8_percent_encode(&json_text(evidence), &QUOTE_SAFE_NONE).to_string();
    if fits.len() <= max_bytes {
        return TruncatedEvidence {
            evidence: evidence.clone(),
            encoded: fits,
            was_truncated: false,
        };
    }
    let truncated = GuardrailValue::Text(TRUNCATED_VALUE.to_owned());
    let marker = utf8_percent_encode(&json_text(&truncated), &QUOTE_SAFE_NONE).to_string();
    TruncatedEvidence {
        evidence: truncated,
        encoded: marker,
        was_truncated: true,
    }
}

#[must_use]
pub fn truncate_evidence_payload(evidence: &GuardrailValue, max_bytes: usize) -> TruncatedEvidence {
    let GuardrailValue::Array(entries) = evidence else {
        return truncate_value(evidence, max_bytes);
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
        shorten_entry(&mut kept, &mut current, max_bytes);
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
