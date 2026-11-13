use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

pub const DEFAULT_ADVISORY_MESSAGE: &str = "The user's latest message was flagged for {reason} by a content safety guardrail. This may be a false positive. Use your judgment: respond helpfully if the request is legitimate, or decline if it is not.";

// False means a structured `input` cannot carry the advisory, so the caller has to block
pub fn inject_advisory_message(data: &mut BTreeMap<String, GuardrailValue>, message: &str) -> bool {
    let advisory = GuardrailValue::Object(BTreeMap::from([
        ("role".to_owned(), GuardrailValue::Text("system".to_owned())),
        (
            "content".to_owned(),
            GuardrailValue::Text(message.to_owned()),
        ),
    ]));
    let text_field = ["instructions", "input"]
        .into_iter()
        .find_map(|key| match data.get(key) {
            Some(GuardrailValue::Text(text)) => Some((key, text.clone())),
            _ => None,
        });
    if let Some((key, text)) = text_field {
        if let Some(GuardrailValue::Array(messages)) = data.get_mut("messages") {
            messages.push(advisory);
        }
        data.insert(
            key.to_owned(),
            GuardrailValue::Text(format!("{text}\n\n{message}")),
        );
        return true;
    }
    if data
        .get("input")
        .is_some_and(|input| *input != GuardrailValue::Null)
    {
        return false;
    }
    if let Some(GuardrailValue::Array(messages)) = data.get_mut("messages") {
        messages.push(advisory);
    } else {
        data.insert("messages".to_owned(), GuardrailValue::Array(vec![advisory]));
    }
    true
}
