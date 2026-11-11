use crate::arguments::json::{Json, Object};
use crate::bedrock_tools::name::{ToolNameMap, make_valid_bedrock_tool_name};
use crate::message_content::{Content, Message, as_fields};

const ASSISTANT: &str = "assistant";
const CONTENT: &str = "content";
const FUNCTION: &str = "function";
const NAME: &str = "name";
const ROLE: &str = "role";
const TOOL_CALLS: &str = "tool_calls";

#[must_use]
pub fn project_message(message: &Message) -> Json {
    let mut fields = message.provider_specific_fields.clone();
    fields.insert(ROLE.to_owned(), Json::Text(message.role.clone()));
    if let Some(name) = &message.name {
        fields.insert(NAME.to_owned(), Json::Text(name.clone()));
    }
    match &message.content {
        None => {}
        Some(Content::Text(text)) => {
            fields.insert(CONTENT.to_owned(), Json::Text(text.clone()));
        }
        Some(Content::Parts(parts)) => {
            let part_fields = parts
                .iter()
                .map(|part| Json::Object(as_fields(part)))
                .collect();
            fields.insert(CONTENT.to_owned(), Json::List(part_fields));
        }
    }
    Json::Object(fields)
}

// LiteLLM sanitizes a tool call's name where it converts the call, so the reverse map fills in that order
#[must_use]
pub fn project_messages(messages: &[Message], names: &mut ToolNameMap) -> Vec<Json> {
    let mut projected = Vec::with_capacity(messages.len());
    for message in messages {
        let mut value = project_message(message);
        if message.role == ASSISTANT
            && let Json::Object(fields) = &mut value
        {
            sanitize_tool_call_names(fields, names);
        }
        projected.push(value);
    }
    projected
}

fn sanitize_tool_call_names(fields: &mut Object, names: &mut ToolNameMap) {
    let Some(Json::List(calls)) = fields.get_mut(TOOL_CALLS) else {
        return;
    };
    for call in calls {
        let Json::Object(call_fields) = call else {
            continue;
        };
        let Some(Json::Object(function)) = call_fields.get_mut(FUNCTION) else {
            continue;
        };
        let Some(Json::Text(name)) = function.get_mut(NAME) else {
            continue;
        };
        *name = make_valid_bedrock_tool_name(name, names);
    }
}
