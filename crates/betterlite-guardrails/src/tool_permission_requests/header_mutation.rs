use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

type Data = BTreeMap<String, GuardrailValue>;

// LiteLLM's add_guardrail_to_applied_guardrails_header: litellm_metadata holds the proxy state when the request has it, else metadata
pub fn record_applied_guardrail(data: &mut Data, guardrail_name: &str) {
    let key = if data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    };
    let mut bucket = match data.remove(key) {
        Some(GuardrailValue::Object(fields)) => fields,
        None | Some(_) => BTreeMap::new(),
    };
    let mut applied = match bucket.remove("applied_guardrails") {
        Some(GuardrailValue::Array(names)) => names,
        None | Some(_) => Vec::new(),
    };
    let name = GuardrailValue::Text(guardrail_name.to_owned());
    if !applied.contains(&name) {
        applied.push(name);
    }
    bucket.insert(
        "applied_guardrails".to_owned(),
        GuardrailValue::Array(applied),
    );
    data.insert(key.to_owned(), GuardrailValue::Object(bucket));
}

mod tests;
