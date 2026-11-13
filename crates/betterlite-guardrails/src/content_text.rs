use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_message_scope::Object;

mod tests;

fn text_of_part(part: &GuardrailValue) -> Option<&str> {
    let GuardrailValue::Object(fields) = part else {
        return None;
    };
    let (Some(GuardrailValue::Text(kind)), Some(GuardrailValue::Text(text))) =
        (fields.get("type"), fields.get("text"))
    else {
        return None;
    };
    (kind == "text").then_some(text.as_str())
}

#[must_use]
pub fn content_to_text(content: &GuardrailValue) -> String {
    if let GuardrailValue::Text(text) = content {
        return text.clone();
    }
    let GuardrailValue::Array(parts) = content else {
        return String::new();
    };
    let texts: Vec<&str> = parts.iter().filter_map(text_of_part).collect();
    texts.join("\n\n")
}

#[must_use]
pub fn is_all_text_parts(content: &GuardrailValue) -> bool {
    let GuardrailValue::Array(parts) = content else {
        return false;
    };
    !parts.is_empty() && parts.iter().all(|part| {
        matches!(
            part,
            GuardrailValue::Object(fields)
                if matches!(fields.get("type"), Some(GuardrailValue::Text(kind)) if kind == "text")
        )
    })
}

// A cache_control breakpoint caches the prefix ending at its part, so the merged row keeps the last one declared.
#[must_use]
pub fn merge_rewritten_text_parts(parts: &[GuardrailValue], new_text: &str) -> Vec<GuardrailValue> {
    let mapping_parts: Vec<&Object> = parts
        .iter()
        .filter_map(|part| {
            let GuardrailValue::Object(fields) = part else {
                return None;
            };
            Some(fields)
        })
        .collect();
    let last_breakpoint = mapping_parts.iter().rev().find_map(|part| {
        part.get("cache_control")
            .filter(|value| !matches!(value, GuardrailValue::Null))
    });
    let mut merged = mapping_parts.first().map_or_else(
        || Object::from([("type".to_owned(), GuardrailValue::Text("text".to_owned()))]),
        |first| (*first).clone(),
    );
    merged.insert("text".to_owned(), GuardrailValue::Text(new_text.to_owned()));
    if let Some(breakpoint) = last_breakpoint {
        merged.insert("cache_control".to_owned(), breakpoint.clone());
    }
    vec![GuardrailValue::Object(merged)]
}
