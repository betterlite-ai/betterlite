use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPlacement {
    pub path: Vec<String>,
    pub part_index: usize,
    pub text: String,
}

#[must_use]
pub fn extract_texts_from_result(result: &BTreeMap<String, GuardrailValue>) -> Vec<TextPlacement> {
    let mut placements = Vec::new();
    if let Some(parts) = result.get("parts") {
        collect_text_parts(parts, &["parts".to_owned()], &mut placements);
    }
    collect_nested_parts(result, &["message"], &mut placements);
    collect_nested_parts(result, &["artifact"], &mut placements);
    collect_nested_parts(result, &["status", "message"], &mut placements);
    collect_artifact_parts(result, &mut placements);
    placements
}

fn collect_nested_parts(
    result: &BTreeMap<String, GuardrailValue>,
    keys: &[&str],
    placements: &mut Vec<TextPlacement>,
) {
    let mut current = result;
    for key in keys {
        let Some(GuardrailValue::Object(container)) = current.get(*key) else {
            return;
        };
        current = container;
    }
    let Some(parts) = current.get("parts") else {
        return;
    };
    let mut path: Vec<String> = keys.iter().map(|key| (*key).to_owned()).collect();
    path.push("parts".to_owned());
    collect_text_parts(parts, &path, placements);
}

fn collect_artifact_parts(
    result: &BTreeMap<String, GuardrailValue>,
    placements: &mut Vec<TextPlacement>,
) {
    let Some(GuardrailValue::Array(artifacts)) = result.get("artifacts") else {
        return;
    };
    for (index, artifact) in artifacts.iter().enumerate() {
        let GuardrailValue::Object(entry) = artifact else {
            continue;
        };
        let Some(parts) = entry.get("parts") else {
            continue;
        };
        let path = [
            "artifacts".to_owned(),
            index.to_string(),
            "parts".to_owned(),
        ];
        collect_text_parts(parts, &path, placements);
    }
}

fn collect_text_parts(
    value: &GuardrailValue,
    path: &[String],
    placements: &mut Vec<TextPlacement>,
) {
    let GuardrailValue::Array(parts) = value else {
        return;
    };
    for (part_index, part) in parts.iter().enumerate() {
        let GuardrailValue::Object(fields) = part else {
            continue;
        };
        if !matches!(fields.get("kind"), Some(GuardrailValue::Text(kind)) if kind == "text") {
            continue;
        }
        let Some(GuardrailValue::Text(text)) = fields.get("text") else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        placements.push(TextPlacement {
            path: path.to_vec(),
            part_index,
            text: text.clone(),
        });
    }
}

pub fn apply_text_to_path(
    result: &mut BTreeMap<String, GuardrailValue>,
    path: &[String],
    part_index: usize,
    text: &str,
) {
    let Some((first, rest)) = path.split_first() else {
        return;
    };
    let Some(mut current) = result.get_mut(first) else {
        return;
    };
    for key in rest {
        let Some(stepped) = index_or_field(current, key) else {
            return;
        };
        current = stepped;
    }
    let GuardrailValue::Array(parts) = current else {
        return;
    };
    let Some(GuardrailValue::Object(part)) = parts.get_mut(part_index) else {
        return;
    };
    part.insert("text".to_owned(), GuardrailValue::Text(text.to_owned()));
}

fn index_or_field<'value>(
    current: &'value mut GuardrailValue,
    key: &str,
) -> Option<&'value mut GuardrailValue> {
    if let Ok(index) = key.parse::<usize>() {
        let GuardrailValue::Array(items) = current else {
            return None;
        };
        return items.get_mut(index);
    }
    let GuardrailValue::Object(fields) = current else {
        return None;
    };
    fields.get_mut(key)
}

mod tests;
