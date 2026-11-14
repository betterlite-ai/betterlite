use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::fields::{get_field, non_empty_text, object_of};

const SCHEMA_SCALAR_KEYS: [&str; 5] = ["name", "description", "title", "const", "default"];
const SCHEMA_LIST_KEYS: [&str; 2] = ["enum", "examples"];
const SCHEMA_EXTRACTED_KEYS: [&str; 7] = [
    "name",
    "description",
    "title",
    "const",
    "default",
    "enum",
    "examples",
];

fn collect_object_texts<'value>(
    object: &'value BTreeMap<String, GuardrailValue>,
    texts: &mut Vec<&'value str>,
    stack: &mut Vec<&'value GuardrailValue>,
) {
    for key in SCHEMA_SCALAR_KEYS {
        if let Some(text) = non_empty_text(object.get(key)) {
            texts.push(text);
        }
    }
    for key in SCHEMA_LIST_KEYS {
        let Some(GuardrailValue::Array(items)) = object.get(key) else {
            continue;
        };
        texts.extend(items.iter().filter_map(|item| non_empty_text(Some(item))));
    }
    for (key, value) in object.iter().rev() {
        if SCHEMA_EXTRACTED_KEYS.contains(&key.as_str()) {
            continue;
        }
        if matches!(value, GuardrailValue::Object(_) | GuardrailValue::Array(_)) {
            stack.push(value);
        }
    }
}

#[must_use]
pub fn schema_text(root: &BTreeMap<String, GuardrailValue>) -> Vec<&str> {
    let mut texts = Vec::new();
    let mut stack: Vec<&GuardrailValue> = Vec::new();
    collect_object_texts(root, &mut texts, &mut stack);
    while let Some(current) = stack.pop() {
        match current {
            GuardrailValue::Object(object) => {
                collect_object_texts(object, &mut texts, &mut stack);
            }
            GuardrailValue::Array(items) => stack.extend(items.iter().rev()),
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_) => {}
        }
    }
    texts
}

#[must_use]
pub fn tool_definition_text(data: &BTreeMap<String, GuardrailValue>) -> Vec<&str> {
    let mut texts = Vec::new();
    if let Some(GuardrailValue::Array(tools)) = data.get("tools") {
        for tool in tools {
            let Some(function) = get_field(Some(tool), "function").and_then(object_of) else {
                continue;
            };
            texts.extend(schema_text(function));
        }
    }
    if let Some(GuardrailValue::Array(functions)) = data.get("functions") {
        for function in functions {
            if let Some(object) = object_of(function) {
                texts.extend(schema_text(object));
            }
        }
    }
    texts
}
