use crate::arguments::json::{Json, Object};
use crate::bedrock_tools::name::{ToolNameMap, make_valid_bedrock_tool_name};
use crate::message_content::{Content, Message, as_fields, is_non_content_values_set};

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

// LiteLLM records the continue message's original names at every blank turn it replaces, in message order
#[must_use]
pub fn project_messages(
    messages: &[Message],
    continue_message: Option<&Json>,
    names: &mut ToolNameMap,
) -> (Vec<Json>, Option<Json>) {
    let mut effective_continue = continue_message.cloned();
    let mut projected = Vec::with_capacity(messages.len());
    for message in messages {
        if swaps_in_continue(message)
            && let Some(Json::Object(fields)) = continue_message
        {
            let mut sanitized = fields.clone();
            sanitize_tool_call_names(&mut sanitized, names);
            effective_continue = Some(Json::Object(sanitized));
        }
        let value = if message.role == ASSISTANT {
            project_message_with_names(message, names)
        } else {
            project_message(message)
        };
        projected.push(value);
    }
    (projected, effective_continue)
}

fn project_message_with_names(message: &Message, names: &mut ToolNameMap) -> Json {
    let mut value = project_message(message);
    if let Json::Object(fields) = &mut value {
        sanitize_tool_call_names(fields, names);
    }
    value
}

// LiteLLM swaps the continue message in only for a blank assistant string with no other set field
#[must_use]
pub fn swaps_in_continue(message: &Message) -> bool {
    if message.role != ASSISTANT {
        return false;
    }
    let Some(Content::Text(text)) = &message.content else {
        return false;
    };
    is_blank_text(text) && !is_non_content_values_set(message)
}

// Python's str.strip() also strips U+001C to U+001F, which Rust's is_whitespace() treats as content
fn is_blank_text(text: &str) -> bool {
    text.trim_matches(|character: char| {
        character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character)
    })
    .is_empty()
}

pub(crate) fn sanitize_tool_call_names(fields: &mut Object, names: &mut ToolNameMap) {
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
