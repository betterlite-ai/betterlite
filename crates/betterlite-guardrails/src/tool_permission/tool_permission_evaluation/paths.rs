use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::super::DEFAULT_MAX_RECURSE_DEPTH;

// LiteLLM's _collect_argument_paths turns nested arguments into dot and [] paths with their leaf values
pub(super) fn collect_argument_paths(
    arguments: &BTreeMap<String, GuardrailValue>,
    collected: &mut BTreeMap<String, Vec<String>>,
) {
    for (key, value) in arguments {
        collect_value(value, key, collected, 1);
    }
}

// LiteLLM compares str(value): a JSON string as its own text and the other scalars as JSON text
fn collect_value(
    value: &GuardrailValue,
    current_path: &str,
    collected: &mut BTreeMap<String, Vec<String>>,
    depth: usize,
) {
    if depth > DEFAULT_MAX_RECURSE_DEPTH {
        return;
    }
    let text = match value {
        GuardrailValue::Object(mapping) => {
            for (key, child) in mapping {
                let next_path = format!("{current_path}.{key}");
                collect_value(child, &next_path, collected, depth.saturating_add(1));
            }
            return;
        }
        GuardrailValue::Array(items) => {
            let list_path = format!("{current_path}[]");
            for item in items {
                collect_value(item, &list_path, collected, depth.saturating_add(1));
            }
            return;
        }
        GuardrailValue::Null => "null".to_owned(),
        GuardrailValue::Boolean(flag) => flag.to_string(),
        GuardrailValue::SignedInteger(number) => number.to_string(),
        GuardrailValue::UnsignedInteger(number) => number.to_string(),
        GuardrailValue::Decimal(number) => serde_json::Number::from_f64(*number)
            .map_or_else(|| "null".to_owned(), |float| float.to_string()),
        GuardrailValue::Text(text) => text.clone(),
    };
    collected
        .entry(current_path.to_owned())
        .or_default()
        .push(text);
}
