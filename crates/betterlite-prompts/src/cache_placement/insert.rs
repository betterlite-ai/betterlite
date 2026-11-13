use crate::arguments::json::{Json, Object};
use crate::cache_accounting::constants::BREAKPOINT_BLOCK_TYPES;
use crate::message_content::{Content, ContentPart, Message};

pub const CACHE_CONTROL: &str = "cache_control";
pub const PROMPT_CACHE_BREAKPOINT: &str = "prompt_cache_breakpoint";
const TEXT: &str = "text";
const MODE: &str = "mode";
const EXPLICIT: &str = "explicit";

#[must_use]
pub fn mark_message(message: &Message, control: &Json, openai_dialect: bool) -> Message {
    if openai_dialect {
        insert_breakpoint(message)
    } else {
        insert_cache_control(message, control)
    }
}

#[must_use]
pub fn breakpoint_marker() -> Json {
    let mut fields = Object::new();
    fields.insert(MODE.to_owned(), Json::Text(EXPLICIT.to_owned()));
    Json::Object(fields)
}

fn insert_cache_control(message: &Message, control: &Json) -> Message {
    let mut marked = message.clone();
    match marked.content.take() {
        Some(Content::Text(text)) => {
            marked
                .provider_specific_fields
                .insert(CACHE_CONTROL.to_owned(), control.clone());
            marked.content = Some(Content::Text(text));
        }
        Some(Content::Parts(mut parts)) => {
            if let Some(last) = parts.last_mut() {
                last.fields
                    .insert(CACHE_CONTROL.to_owned(), control.clone());
            }
            marked.content = Some(Content::Parts(parts));
        }
        None => {}
    }
    marked
}

fn insert_breakpoint(message: &Message) -> Message {
    if message.role == "assistant" {
        return message.clone();
    }
    let mut marked = message.clone();
    match marked.content.take() {
        Some(Content::Text(text)) => {
            let mut fields = Object::new();
            fields.insert(TEXT.to_owned(), Json::Text(text));
            fields.insert(PROMPT_CACHE_BREAKPOINT.to_owned(), breakpoint_marker());
            marked.content = Some(Content::Parts(vec![ContentPart {
                kind: TEXT.to_owned(),
                fields,
            }]));
        }
        Some(Content::Parts(mut parts)) => {
            let target = parts
                .iter_mut()
                .rev()
                .find(|part| BREAKPOINT_BLOCK_TYPES.contains(&part.kind.as_str()));
            if let Some(block) = target {
                block
                    .fields
                    .insert(PROMPT_CACHE_BREAKPOINT.to_owned(), breakpoint_marker());
            }
            marked.content = Some(Content::Parts(parts));
        }
        None => {}
    }
    marked
}
