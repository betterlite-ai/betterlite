use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;

use super::lifecycle::BlockCodeExecutionGuardrail;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::GuardrailFailure;

pub const EXECUTION_REQUEST_LANGUAGE: &str = "execution_request";

const BLOCKED_STATUS: u16 = 400;

#[must_use]
pub fn block_message(language: &str) -> String {
    if language == EXECUTION_REQUEST_LANGUAGE {
        "Content blocked: execution request detected".to_owned()
    } else {
        format!("Content blocked: executable code block detected (language: {language})")
    }
}

#[must_use]
pub fn block_failure(
    guardrail: &BlockCodeExecutionGuardrail,
    language: &str,
    input_type: GuardrailInputType,
    request_data: &BTreeMap<String, GuardrailValue>,
) -> GuardrailFailure {
    let message = block_message(language);
    match input_type {
        GuardrailInputType::Request => {
            let detection_info = BTreeMap::from([(
                "language".to_owned(),
                GuardrailValue::Text(language.to_owned()),
            )]);
            guardrail
                .base
                .passthrough_failure(&message, request_data, Some(detection_info), None)
        }
        GuardrailInputType::Response => GuardrailFailure::HttpObject {
            status: BLOCKED_STATUS,
            detail: response_detail(guardrail, language, message),
        },
    }
}

fn response_detail(
    guardrail: &BlockCodeExecutionGuardrail,
    language: &str,
    message: String,
) -> IndexMap<String, GuardrailValue> {
    let name = guardrail
        .base
        .settings
        .guardrail_name
        .clone()
        .map_or(GuardrailValue::Null, GuardrailValue::Text);
    [
        ("error".to_owned(), GuardrailValue::Text(message)),
        ("guardrail".to_owned(), name),
        (
            "language".to_owned(),
            GuardrailValue::Text(language.to_owned()),
        ),
    ]
    .into_iter()
    .collect()
}
