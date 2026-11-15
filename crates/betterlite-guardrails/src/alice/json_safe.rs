use std::ptr;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

pub(crate) const MAX_DEPTH: usize = 12;
pub(crate) const MAX_ITEMS: usize = 5000;

// LiteLLM decides what is dropped by a rule, not a field list, so a new nesting path can never reintroduce a credential
pub(crate) const CREDENTIAL_KEYS_TO_STRIP: [&str; 5] = [
    "secret_fields",
    "api_key",
    "raw_headers",
    "headers",
    "provider_specific_header",
];

pub(crate) fn json_safe(
    value: &GuardrailValue,
    depth: usize,
    strip_keys: &[&str],
    seen: &mut Vec<*const GuardrailValue>,
) -> GuardrailValue {
    match value {
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_) => value.clone(),
        GuardrailValue::Array(items) => {
            if depth >= MAX_DEPTH || seen.iter().any(|entry| ptr::eq(*entry, value)) {
                return GuardrailValue::Null;
            }
            seen.push(ptr::from_ref(value));
            let kept = items
                .iter()
                .take(MAX_ITEMS)
                .map(|item| json_safe(item, depth.saturating_add(1), strip_keys, seen))
                .collect();
            seen.pop();
            GuardrailValue::Array(kept)
        }
        GuardrailValue::Object(map) => {
            if depth >= MAX_DEPTH || seen.iter().any(|entry| ptr::eq(*entry, value)) {
                return GuardrailValue::Null;
            }
            seen.push(ptr::from_ref(value));
            let kept = map
                .iter()
                .take(MAX_ITEMS)
                .filter(|(key, _value)| !strip_keys.contains(&key.as_str()))
                .map(|(key, item)| {
                    (
                        key.clone(),
                        json_safe(item, depth.saturating_add(1), strip_keys, seen),
                    )
                })
                .collect();
            seen.pop();
            GuardrailValue::Object(kept)
        }
    }
}

mod tests;
