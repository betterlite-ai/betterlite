use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::azure::convert::convert_to_azure_openai_messages;
use betterlite_prompts::azure::stringify::stringify_json_tool_call_content;
use betterlite_prompts::azure::text::azure_text_pt;
use betterlite_prompts::cohere::history::cohere_messages_pt_v2;
use betterlite_prompts::cohere::prompt::cohere_message_pt;
use betterlite_prompts::message_content::{Content, Message};
use serde::Deserialize;
use serde_json::{Value, json, to_value};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", deny_unknown_fields)]
pub(crate) enum Input {
    #[serde(rename = "convert_to_azure_openai_messages")]
    AzureMessages { messages: Vec<Message> },
    #[serde(rename = "azure_text_pt")]
    AzureText { messages: Vec<Message> },
    #[serde(rename = "stringify_json_tool_call_content")]
    Stringify { messages: Vec<Message> },
    #[serde(rename = "cohere_messages")]
    Cohere { messages: Vec<Message> },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::AzureMessages { messages } => {
                let converted = convert_to_azure_openai_messages(messages)
                    .map_err(|error| helper_error("AzureError", error.to_string()))?;
                ok_value(converted)
            }
            Input::AzureText { messages } => {
                let prompt = azure_text_pt(messages)
                    .map_err(|error| helper_error("AzureError", error.to_string()))?;
                ok_value(prompt)
            }
            Input::Stringify { messages } => {
                let converted = stringify_json_tool_call_content(messages)
                    .map_err(|error| helper_error("AzureError", error.to_string()))?;
                stringified_values(&converted)
            }
            Input::Cohere { messages } => cohere_values(messages),
        }
    }
}

fn stringified_values(messages: &[Message]) -> HelperResult {
    let mut values = Vec::with_capacity(messages.len());
    for message in messages {
        let mut value = to_value(message).map_err(|error| serialize_error(&error))?;
        if message.role == "tool"
            && let Some(Content::Text(text)) = &message.content
        {
            let parsed: Json =
                serde_json::from_str(text).map_err(|error| serialize_error(&error))?;
            let parsed_value = to_value(parsed).map_err(|error| serialize_error(&error))?;
            if let Some(fields) = value.as_object_mut() {
                fields.insert("content".to_owned(), parsed_value);
            }
        }
        values.push(value);
    }
    Ok(without_nulls(Value::Array(values)))
}

fn cohere_values(messages: &[Message]) -> HelperResult {
    let v2 = cohere_messages_pt_v2(messages)
        .map_err(|error| helper_error("CohereError", error.to_string()))?;
    let legacy = cohere_message_pt(messages)
        .map_err(|error| helper_error("CohereError", error.to_string()))?;
    Ok(without_nulls(json!([
        {"message": v2.message, "chat_history": v2.chat_history},
        {"prompt": legacy.prompt, "tool_results": legacy.tool_results}
    ])))
}

// betterlite's message model cannot tell an absent key from a null one, so both sides drop null fields
fn without_nulls(value: Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .filter(|(_key, field)| !field.is_null())
                .map(|(key, field)| (key, without_nulls(field)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(without_nulls).collect()),
        other @ (Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)) => other,
    }
}

fn ok_value<Payload>(payload: Payload) -> HelperResult
where
    Payload: serde::Serialize,
{
    to_value(payload).map_err(|error| serialize_error(&error))
}

fn serialize_error(error: &serde_json::Error) -> Box<HelperError> {
    helper_error("SerializationError", error.to_string())
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
