use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

pub(super) const PRE_CALL_CONTENT_KEYS: [&str; 9] = [
    "messages",
    "input",
    "prompt",
    "system",
    "instructions",
    "tools",
    "functions",
    "function_call",
    "tool_choice",
];
const STREAM_CONTROL_KEY: &str = "stream_holdback_chars";

// LiteLLM's _inputs_were_modified: a key missing on one side equals null on the other
fn inputs_were_modified(
    baseline: &BTreeMap<String, GuardrailValue>,
    response: &BTreeMap<String, GuardrailValue>,
) -> bool {
    let keys: BTreeSet<&String> = baseline.keys().chain(response.keys()).collect();
    keys.into_iter()
        .filter(|key| key.as_str() != STREAM_CONTROL_KEY)
        .any(|key| {
            baseline.get(key).unwrap_or(&GuardrailValue::Null)
                != response.get(key).unwrap_or(&GuardrailValue::Null)
        })
}

// LiteLLM's _summarize_guardrail_response: a prompt never reaches the logs, only allow or mask
#[must_use]
pub fn summarize_guardrail_response(
    response: Option<&GuardrailValue>,
    baseline: Option<&BTreeMap<String, GuardrailValue>>,
    event_type: Option<GuardrailEventHooks>,
) -> GuardrailValue {
    let Some(present) = response.filter(|value| **value != GuardrailValue::Null) else {
        return GuardrailValue::Object(BTreeMap::new());
    };
    let (Some(before), GuardrailValue::Object(returned)) = (baseline, present) else {
        return present.clone();
    };
    let modified = if event_type == Some(GuardrailEventHooks::PreCall) {
        let content: BTreeMap<String, GuardrailValue> = returned
            .iter()
            .filter(|(key, _value)| PRE_CALL_CONTENT_KEYS.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        inputs_were_modified(before, &content)
    } else {
        inputs_were_modified(before, returned)
    };
    GuardrailValue::Text(if modified { "mask" } else { "allow" }.to_owned())
}
