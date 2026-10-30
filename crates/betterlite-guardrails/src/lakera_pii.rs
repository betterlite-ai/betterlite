use std::collections::BTreeMap;

use serde_json::{Value, json};

mod tests;

#[must_use]
pub fn mask_pii_in_messages(
    messages: &[Value],
    response: Option<&Value>,
    counts: &mut BTreeMap<String, usize>,
) -> Vec<Value> {
    let Some(payload) = response
        .and_then(|value| value.get("payload"))
        .and_then(Value::as_array)
    else {
        return messages.to_vec();
    };
    messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let Some(content) = message.get("content").and_then(Value::as_str) else {
                return message.clone();
            };
            let message_id = u64::try_from(index).map_or(u64::MAX, |value| value);
            let mut replacements = payload
                .iter()
                .filter(|item| item.get("message_id").and_then(Value::as_u64) == Some(message_id))
                .collect::<Vec<_>>();
            replacements.sort_by_key(|item| {
                std::cmp::Reverse((
                    item.get("start").and_then(Value::as_u64).unwrap_or(0),
                    item.get("end").and_then(Value::as_u64).unwrap_or(0),
                ))
            });
            let mut masked = content.to_owned();
            for item in replacements {
                let Some(kind) = item
                    .get("detector_type")
                    .and_then(Value::as_str)
                    .filter(|kind| !kind.is_empty())
                else {
                    continue;
                };
                let label = kind.rsplit('/').next().unwrap_or("PII").to_uppercase();
                let start = usize::try_from(item.get("start").and_then(Value::as_u64).unwrap_or(0))
                    .unwrap_or(0);
                let end = usize::try_from(item.get("end").and_then(Value::as_u64).unwrap_or(0))
                    .unwrap_or(start);
                if start <= end
                    && end <= masked.len()
                    && masked.is_char_boundary(start)
                    && masked.is_char_boundary(end)
                {
                    masked.replace_range(start..end, &format!("[MASKED {label}]"));
                    *counts.entry(label).or_default() += 1;
                }
            }
            let mut result = message.as_object().cloned().unwrap_or_default();
            result.insert("content".to_owned(), Value::String(masked));
            Value::Object(result)
        })
        .collect()
}

#[must_use]
pub fn is_only_pii_violation(response: Option<&Value>) -> bool {
    let Some(breakdown) = response
        .and_then(|value| value.get("breakdown"))
        .and_then(Value::as_array)
    else {
        return false;
    };
    let detected = breakdown
        .iter()
        .filter(|item| item.get("detected").and_then(Value::as_bool) == Some(true))
        .collect::<Vec<_>>();
    !detected.is_empty()
        && detected.iter().all(|item| {
            item.get("detector_type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.starts_with("pii/"))
        })
}

#[must_use]
pub fn blocked_exception(response: Option<&Value>) -> Value {
    let detail = json!({"error":"Violated guardrail policy","lakera_guardrail_response":response});
    json!({"status_code":400,"detail":detail})
}
