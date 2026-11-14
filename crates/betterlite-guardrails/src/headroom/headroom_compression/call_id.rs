use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::request_data::{self, CALL_ID_FIELD};
use crate::guardrail_base::inputs::GuardrailLogTarget;

pub(crate) fn resolve(
    logging_obj: Option<&GuardrailLogTarget>,
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Option<String> {
    if let Some(call_id) = logging_call_id(logging_obj) {
        return Some(call_id.to_owned());
    }
    request_data::call_id(request_data).map(str::to_owned)
}

fn logging_call_id(logging_obj: Option<&GuardrailLogTarget>) -> Option<&str> {
    let target = logging_obj?;
    [
        target.model_call_params.as_ref(),
        target.litellm_params.as_ref(),
    ]
    .into_iter()
    .flatten()
    .find_map(call_id_in)
}

fn call_id_in(params: &BTreeMap<String, GuardrailValue>) -> Option<&str> {
    match params.get(CALL_ID_FIELD) {
        Some(GuardrailValue::Text(call_id)) if !call_id.is_empty() => Some(call_id.as_str()),
        _ => None,
    }
}
