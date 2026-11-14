use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::{Content, Message};
use betterlite_prompts::message_fields::last_user::get_last_user_message;

use super::error::AzureContentSafetyError;

const RESPONSES_CALL_TYPES: [&str; 2] = ["responses", "aresponses"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemSelection {
    EveryItem,
    ItemsWithRole,
}

pub fn get_user_prompt_from_request(
    data: &BTreeMap<String, GuardrailValue>,
    call_type: &str,
) -> Result<Option<String>, AzureContentSafetyError> {
    if RESPONSES_CALL_TYPES.contains(&call_type) {
        return match data.get("input") {
            Some(GuardrailValue::Text(text)) => Ok(get_last_user_message(&[Message {
                role: "user".to_owned(),
                content: Some(Content::Text(text.clone())),
                name: None,
                provider_specific_fields: Object::new(),
            }])),
            Some(GuardrailValue::Array(items)) => {
                messages_from_items(items, "input", call_type, ItemSelection::ItemsWithRole)
                    .map(|messages| get_last_user_message(&messages))
            }
            None | Some(_) => Ok(None),
        };
    }
    match data.get("messages") {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Array(items)) => {
            messages_from_items(items, "messages", call_type, ItemSelection::EveryItem)
                .map(|messages| get_last_user_message(&messages))
        }
        Some(_) => Err(AzureContentSafetyError::MessagesShape {
            call_type: call_type.to_owned(),
        }),
    }
}

fn messages_from_items(
    items: &[GuardrailValue],
    field: &'static str,
    call_type: &str,
    selection: ItemSelection,
) -> Result<Vec<Message>, AzureContentSafetyError> {
    let mut messages = Vec::with_capacity(items.len());
    for item in items {
        if selection == ItemSelection::ItemsWithRole
            && !matches!(item, GuardrailValue::Object(object) if object.contains_key("role"))
        {
            continue;
        }
        messages.push(message_from_value(item, field, call_type)?);
    }
    Ok(messages)
}

fn message_from_value(
    value: &GuardrailValue,
    field: &'static str,
    call_type: &str,
) -> Result<Message, AzureContentSafetyError> {
    let json =
        serde_json::to_value(value).map_err(|source| AzureContentSafetyError::MessageJson {
            field,
            call_type: call_type.to_owned(),
            source,
        })?;
    serde_json::from_value(json).map_err(|source| AzureContentSafetyError::MessageJson {
        field,
        call_type: call_type.to_owned(),
        source,
    })
}
