use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::generic_guardrail_contracts::{GuardrailExtraValue, GuardrailMessage};
use crate::grayswan::ReasoningMode;
use crate::grayswan::wire::{GraySwanInput, GraySwanResponse};

fn as_text(value: &GuardrailExtraValue) -> Option<&str> {
    match value {
        GuardrailExtraValue::Text(text) => Some(text),
        GuardrailExtraValue::Null
        | GuardrailExtraValue::Boolean(_)
        | GuardrailExtraValue::SignedInteger(_)
        | GuardrailExtraValue::UnsignedInteger(_)
        | GuardrailExtraValue::Decimal(_)
        | GuardrailExtraValue::Array(_)
        | GuardrailExtraValue::Object(_) => None,
    }
}

pub(super) fn category_override(
    metadata: &BTreeMap<String, GuardrailExtraValue>,
) -> Option<BTreeMap<String, String>> {
    let GuardrailExtraValue::Object(categories) = metadata.get("categories")? else {
        return None;
    };
    let mut result = BTreeMap::new();
    for (name, value) in categories {
        if let Some(text) = as_text(value) {
            result.insert(name.clone(), text.to_owned());
        }
    }
    Some(result)
}

pub(super) fn text_override(
    metadata: &BTreeMap<String, GuardrailExtraValue>,
    name: &str,
) -> Option<String> {
    let value = metadata.get(name)?;
    as_text(value).map(str::to_owned)
}

const REASONING_MODE_NAMES: [(&str, ReasoningMode); 3] = [
    ("off", ReasoningMode::Off),
    ("hybrid", ReasoningMode::Hybrid),
    ("thinking", ReasoningMode::Thinking),
];

pub(super) fn reasoning_mode_override(
    metadata: &BTreeMap<String, GuardrailExtraValue>,
) -> Option<ReasoningMode> {
    let text = text_override(metadata, "reasoning_mode")?.to_ascii_lowercase();
    REASONING_MODE_NAMES
        .iter()
        .find(|(name, _)| *name == text)
        .map(|(_, mode)| *mode)
}

pub(super) fn conversation_messages(input: &GraySwanInput) -> Vec<GuardrailMessage> {
    let mut messages = input.request_history.clone();
    messages.extend(input.messages.clone());
    messages
}

pub(super) fn format_violation_message(response: &GraySwanResponse, is_output: bool) -> String {
    let location = if is_output {
        "model response"
    } else {
        "request"
    };
    let mut message = format!(
        "Gray Swan Cygnal flagged the {location} with a violation score of {:.2}.",
        response.violation
    );
    if !response.violated_rule_descriptions.is_empty() {
        let rules = response
            .violated_rule_descriptions
            .iter()
            .map(describe_violated_rule)
            .collect::<Vec<_>>()
            .join(", ");
        if write!(message, " It violated rule(s): {rules}.").is_err() {
            return message;
        }
    }
    if response.mutation == Some(true) {
        message.push_str(" A mutation disguising harmful intent was detected.");
    }
    if response.ipi == Some(true) {
        message.push_str(" An indirect prompt injection was detected.");
    }
    message
}

fn describe_violated_rule(rule: &GuardrailExtraValue) -> String {
    if let GuardrailExtraValue::Object(fields) = rule {
        match (
            text_override(fields, "name"),
            text_override(fields, "description"),
        ) {
            (Some(matched_name), Some(matched_description)) => {
                return format!("{matched_name}: {matched_description}");
            }
            (Some(matched_name), None) => return matched_name,
            _ => {}
        }
    }
    if let GuardrailExtraValue::Text(text) = rule {
        return text.clone();
    }
    format!("{rule:?}")
}
