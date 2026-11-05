use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::RATIONALE_TRUNCATE_CHARS;
use super::wire::ScanResult;

const UNKNOWN: &str = "unknown";

#[must_use]
pub fn scan_block_message(result: &ScanResult) -> String {
    let violations = match &result.xecguard_result {
        Some(GuardrailValue::Array(items)) => items.as_slice(),
        _ => &[],
    };
    let mut names: Vec<&str> = Vec::new();
    let mut rationale = String::new();
    for violation in violations {
        let GuardrailValue::Object(fields) = violation else {
            continue;
        };
        if let Some(name) = fields.get("violated_policy_name").and_then(text_value)
            && !name.is_empty()
            && !names.contains(&name)
        {
            names.push(name);
        }
        if rationale.is_empty()
            && let Some(candidate) = non_empty_rationale(fields)
        {
            rationale = candidate;
        }
    }
    let policies = if names.is_empty() {
        UNKNOWN.to_owned()
    } else {
        names.join(",")
    };
    format!(
        "Blocked by XecGuard: policies=[{policies}] trace_id={} rationale={rationale}",
        result.trace_id_text()
    )
}

#[must_use]
pub fn grounding_block_message(result: &ScanResult) -> String {
    let trace_id = result.trace_id_text();
    let Some(GuardrailValue::Object(fields)) = &result.xecguard_result else {
        return format!(
            "Blocked by XecGuard grounding: rules=[{UNKNOWN}] trace_id={trace_id} rationale="
        );
    };
    let rules = grounding_rules(fields);
    let rules_text = if rules.is_empty() {
        UNKNOWN.to_owned()
    } else {
        rules.join(",")
    };
    format!(
        "Blocked by XecGuard grounding: rules=[{rules_text}] trace_id={trace_id} rationale={}",
        grounding_rationale(fields)
    )
}

fn text_value(value: &GuardrailValue) -> Option<&str> {
    if let GuardrailValue::Text(text) = value {
        Some(text)
    } else {
        None
    }
}

fn non_empty_rationale(fields: &BTreeMap<String, GuardrailValue>) -> Option<String> {
    fields
        .get("rationale")
        .and_then(text_value)
        .filter(|rationale| !rationale.is_empty())
        .map(|rationale| rationale.chars().take(RATIONALE_TRUNCATE_CHARS).collect())
}

fn grounding_rules(fields: &BTreeMap<String, GuardrailValue>) -> Vec<&str> {
    match fields.get("violated_rules_list") {
        Some(GuardrailValue::Array(items)) => items.iter().filter_map(text_value).collect(),
        _ => Vec::new(),
    }
}

fn grounding_rationale(fields: &BTreeMap<String, GuardrailValue>) -> String {
    fields
        .get("rationale")
        .and_then(text_value)
        .map(|rationale| rationale.chars().take(RATIONALE_TRUNCATE_CHARS).collect())
        .unwrap_or_default()
}

mod tests;
