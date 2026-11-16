use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::Data;
use super::metadata::bucket_key;

const APPLIED_KEY: &str = "applied_guardrails";

// LiteLLM's add_guardrail_to_applied_guardrails_header, called before any block
pub(super) fn record(data: &mut Data, guardrail_name: Option<&str>) {
    let Some(name) = guardrail_name else {
        return;
    };
    let key = bucket_key(data);
    let mut bucket = match data.remove(key) {
        Some(GuardrailValue::Object(fields)) => fields,
        _ => BTreeMap::new(),
    };
    let mut applied = match bucket.remove(APPLIED_KEY) {
        None | Some(GuardrailValue::Null) => Vec::new(),
        Some(GuardrailValue::Array(names)) => names,
        Some(_) => {
            tracing::warn!("Model Armor: replacing an applied_guardrails value that is not a list");
            Vec::new()
        }
    };
    if !applied
        .iter()
        .any(|existing| matches!(existing, GuardrailValue::Text(text) if text == name))
    {
        applied.push(GuardrailValue::Text(name.to_owned()));
    }
    bucket.insert(APPLIED_KEY.to_owned(), GuardrailValue::Array(applied));
    data.insert(key.to_owned(), GuardrailValue::Object(bucket));
}

mod tests;
