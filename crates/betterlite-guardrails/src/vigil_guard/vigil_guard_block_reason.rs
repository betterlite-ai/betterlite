use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::BLOCK_REASON_MAX_CHARS;
use super::vigil_guard_analysis::Analysis;

const DEFAULT_BLOCK_REASON: &str = "Blocked by policy";

#[must_use]
pub fn build_block_reason(analysis: &Analysis) -> String {
    for value in [
        analysis.block_message.as_ref(),
        analysis.decision_reason.as_ref(),
    ] {
        if let Some(GuardrailValue::Text(text)) = value {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return trimmed.chars().take(BLOCK_REASON_MAX_CHARS).collect();
            }
        }
    }
    let Some(GuardrailValue::Array(categories)) = analysis.categories.as_ref() else {
        return DEFAULT_BLOCK_REASON.to_owned();
    };
    let names: Vec<&str> = categories
        .iter()
        .filter_map(|category| match category {
            GuardrailValue::Text(name) if !name.trim().is_empty() => Some(name.as_str()),
            GuardrailValue::Text(_)
            | GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Array(_)
            | GuardrailValue::Object(_) => None,
        })
        .collect();
    if names.is_empty() {
        return DEFAULT_BLOCK_REASON.to_owned();
    }
    names
        .join(", ")
        .chars()
        .take(BLOCK_REASON_MAX_CHARS)
        .collect()
}
