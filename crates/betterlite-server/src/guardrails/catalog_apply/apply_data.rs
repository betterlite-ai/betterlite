use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::routes::ApplyGuardrailRequest;

use super::ports::Body;

// LiteLLM's data dict and the request_data handed to the guardrail in the apply route
pub(super) fn call_data(request: &ApplyGuardrailRequest) -> Body {
    let messages = request.messages.as_deref().unwrap_or_default();
    let route = GuardrailValue::Text("/apply_guardrail".to_owned());
    let metadata = BTreeMap::from([("route".to_owned(), route)]);
    let mut data = Body::new();
    data.insert(
        "guardrail_name".to_owned(),
        GuardrailValue::Text(request.guardrail_name.clone()),
    );
    data.insert(
        "input".to_owned(),
        GuardrailValue::Array(vec![GuardrailValue::Text(request.text.clone())]),
    );
    data.insert(
        "messages".to_owned(),
        GuardrailValue::Array(message_values(messages)),
    );
    data.insert("metadata".to_owned(), GuardrailValue::Object(metadata));
    data
}

pub(super) fn sent_request_data(request: &ApplyGuardrailRequest) -> Body {
    let mut sent = Body::new();
    if let Some(messages) = &request.messages {
        sent.insert(
            "messages".to_owned(),
            GuardrailValue::Array(message_values(messages)),
        );
    }
    if let Some(metadata) = &request.metadata {
        sent.insert(
            "metadata".to_owned(),
            GuardrailValue::Object(metadata.0.clone()),
        );
    }
    sent
}

fn message_values(messages: &[OpaqueParams]) -> Vec<GuardrailValue> {
    messages
        .iter()
        .map(|message| GuardrailValue::Object(message.0.clone()))
        .collect()
}
