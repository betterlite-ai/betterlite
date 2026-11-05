use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::RATIONALE_TRUNCATE_CHARS;
use super::wire::ScanResult;

#[must_use]
pub fn scan_block_message(result: &ScanResult) -> String {
    let violations = match &result.xecguard_result {
        Some(GuardrailValue::Array(items)) => items.as_slice(),
        _ => &[],
    };
    let policies = unique_policies(violations);
    let rationale = violations
        .iter()
        .find_map(non_empty_rationale)
        .unwrap_or_default();
    format!(
        "Blocked by XecGuard: policies=[{policies}] trace_id={} rationale={rationale}",
        result.trace_id_text()
    )
}

#[must_use]
pub fn grounding_block_message(result: &ScanResult) -> String {
    let (rules, rationale) = match &result.xecguard_result {
        Some(GuardrailValue::Object(fields)) => {
            (grounding_rules(fields), grounding_rationale(fields))
        }
        _ => (Vec::new(), String::new()),
    };
    let rules_text = if rules.is_empty() {
        "unknown".to_owned()
    } else {
        rules.join(",")
    };
    format!(
        "Blocked by XecGuard grounding: rules=[{rules_text}] trace_id={} rationale={rationale}",
        result.trace_id_text()
    )
}

fn text_value(value: &GuardrailValue) -> Option<&str> {
    if let GuardrailValue::Text(text) = value {
        Some(text)
    } else {
        None
    }
}

fn unique_policies(violations: &[GuardrailValue]) -> String {
    let mut names: Vec<&str> = Vec::new();
    for violation in violations {
        let GuardrailValue::Object(fields) = violation else {
            continue;
        };
        let Some(name) = fields.get("violated_policy_name").and_then(text_value) else {
            continue;
        };
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    if names.is_empty() {
        "unknown".to_owned()
    } else {
        names.join(",")
    }
}

fn non_empty_rationale(violation: &GuardrailValue) -> Option<String> {
    let GuardrailValue::Object(fields) = violation else {
        return None;
    };
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
