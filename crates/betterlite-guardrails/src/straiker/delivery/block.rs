use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::{GuardrailFailure, ModifyResponseFailure};
use crate::straiker::delivery::DEFAULT_BLOCK_MESSAGE;

const BLOCK_STATUS: u16 = 400;

#[must_use]
pub fn block_message(reason: Option<String>) -> String {
    let fallback = DEFAULT_BLOCK_MESSAGE.to_owned();
    reason.unwrap_or(fallback)
}

#[must_use]
pub fn block_failure(
    guardrail_name: &str,
    request_data: &BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
    message: &str,
    blocked_content: bool,
) -> GuardrailFailure {
    match input_type {
        GuardrailInputType::Request => GuardrailFailure::GuardrailRaised {
            guardrail_name: Some(guardrail_name.to_owned()),
            message: message.to_owned(),
            wrap_default_message: false,
            status_code: BLOCK_STATUS,
            blocked_content,
        },
        GuardrailInputType::Response => {
            let model = match request_data.get("model") {
                Some(GuardrailValue::Text(name)) => name.clone(),
                Some(_) | None => "unknown".to_owned(),
            };
            GuardrailFailure::ModifyResponse(Box::new(ModifyResponseFailure {
                message: message.to_owned(),
                model,
                request_data: request_data.clone(),
                guardrail_name: Some(guardrail_name.to_owned()),
                detection_info: BTreeMap::new(),
                original_response: request_data.get("response").cloned(),
            }))
        }
    }
}

// LiteLLM's hidden.setdefault("straiker", {}).update({action, turn_id}) on the response object.
pub fn record_verdict(
    request_data: &mut BTreeMap<String, GuardrailValue>,
    action: &str,
    turn_id: Option<&str>,
) {
    let Some(GuardrailValue::Object(response)) = request_data.get_mut("response") else {
        return;
    };
    let Some(GuardrailValue::Object(hidden)) = response.get_mut("_hidden_params") else {
        return;
    };
    let entry = hidden
        .entry("straiker".to_owned())
        .or_insert_with(|| GuardrailValue::Object(BTreeMap::new()));
    let GuardrailValue::Object(straiker) = entry else {
        return;
    };
    straiker.insert("action".to_owned(), GuardrailValue::Text(action.to_owned()));
    straiker.insert(
        "turn_id".to_owned(),
        turn_id.map_or(GuardrailValue::Null, |text| {
            GuardrailValue::Text(text.to_owned())
        }),
    );
}
