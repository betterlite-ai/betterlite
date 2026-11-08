use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::content_inspection::{
    NON_CONVERSATIONAL_CALL_TYPES, TEXT_CONTENT_CALL_TYPES, apply_redacted_messages_back,
    build_inspection_messages, has_non_string_content, is_string_batch_input, iter_message_text,
};
use serde::Deserialize;
use serde_json::json;

type Data = BTreeMap<String, GuardrailValue>;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Function {
    IterMessageText {
        data: Data,
    },
    BuildInspectionMessages {
        data: Data,
    },
    IsStringBatchInput {
        data: Data,
    },
    HasNonStringContent {
        data: Data,
    },
    ApplyRedactedMessagesBack {
        data: Data,
        redacted_messages: Vec<GuardrailValue>,
    },
    CallTypes {
        call_type: String,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Function;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Function::IterMessageText { data } => Ok(json!({"result": iter_message_text(data)})),
            Function::BuildInspectionMessages { data } => {
                Ok(json!({"result": build_inspection_messages(data)}))
            }
            Function::IsStringBatchInput { data } => {
                Ok(json!({"result": is_string_batch_input(data)}))
            }
            Function::HasNonStringContent { data } => {
                Ok(json!({"result": has_non_string_content(data)}))
            }
            Function::ApplyRedactedMessagesBack {
                data,
                redacted_messages,
            } => {
                let mut rewritten = data.clone();
                let returned = apply_redacted_messages_back(&mut rewritten, redacted_messages);
                Ok(json!({"returned": returned, "data": rewritten}))
            }
            Function::CallTypes { call_type } => Ok(json!({
                "text_content": TEXT_CONTENT_CALL_TYPES.contains(&call_type.as_str()),
                "non_conversational": NON_CONVERSATIONAL_CALL_TYPES.contains(&call_type.as_str()),
            })),
        }
    }
}
