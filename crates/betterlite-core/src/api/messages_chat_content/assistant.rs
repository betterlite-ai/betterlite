use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json as ModelJson;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::ContentPart;
use betterlite_prompts::reasoning_fields::thinking_text::reasoning_content_from_blocks;

use super::error::ChatContentError;
use super::{ChatTarget, text_part};
use crate::api::messages_content_primitives::markers::copy_cache_control;
use crate::api::messages_content_primitives::signatures::tool_use_signature;
use crate::api::messages_content_primitives::tools::truncate_tool_name;

#[derive(Default)]
struct Parts {
    text: Option<String>,
    content: Vec<Json>,
    cached_text: bool,
    tool_calls: Vec<Json>,
    thinking: Vec<ContentPart>,
}

fn thinking_fields(block: &Object) -> Object {
    let text = block
        .get("thinking")
        .and_then(Json::as_str)
        .unwrap_or_default();
    let signature = block
        .get("signature")
        .and_then(Json::as_str)
        .unwrap_or_default();
    let mut fields = Object::new();
    fields.insert("thinking".to_owned(), Json::Text(text.to_owned()));
    fields.insert("signature".to_owned(), Json::Text(signature.to_owned()));
    fields
}

fn redacted_fields(block: &Object) -> Object {
    let data = block.get("data").and_then(Json::as_str).unwrap_or_default();
    let mut fields = Object::new();
    fields.insert("data".to_owned(), Json::Text(data.to_owned()));
    fields
}

fn tool_call(block: &Object, model: Option<&str>) -> Json {
    let name = block.get("name").and_then(Json::as_str).unwrap_or_default();
    let id = block.get("id").and_then(Json::as_str).unwrap_or_default();
    let absent_input = Json::Object(Object::new());
    let input = block.get("input").unwrap_or(&absent_input);
    let mut function = Object::new();
    function.insert("name".to_owned(), Json::Text(truncate_tool_name(name)));
    function.insert(
        "arguments".to_owned(),
        Json::Text(dumps(&ModelJson::from(input))),
    );
    if let Some(signature) = tool_use_signature(block).filter(|signature| !signature.is_empty()) {
        let mut specific = Object::new();
        specific.insert(
            "thought_signature".to_owned(),
            Json::Text(signature.to_owned()),
        );
        function.insert(
            "provider_specific_fields".to_owned(),
            Json::Object(specific),
        );
    }
    let mut call = Object::new();
    call.insert("id".to_owned(), Json::Text(id.to_owned()));
    call.insert("type".to_owned(), Json::Text("function".to_owned()));
    call.insert("function".to_owned(), Json::Object(function));
    copy_cache_control(block, &mut call, model);
    Json::Object(call)
}

fn push_object_block(parts: &mut Parts, block: &Object, model: Option<&str>) {
    match block.get("type").and_then(Json::as_str) {
        Some("text") => {
            let mut part = text_part(block);
            copy_cache_control(block, &mut part, model);
            parts.cached_text = parts.cached_text || part.contains_key("cache_control");
            parts.content.push(Json::Object(part));
        }
        Some("tool_use") => parts.tool_calls.push(tool_call(block, model)),
        Some("thinking") => parts.thinking.push(ContentPart {
            kind: "thinking".to_owned(),
            fields: thinking_fields(block),
        }),
        Some("redacted_thinking") => parts.thinking.push(ContentPart {
            kind: "redacted_thinking".to_owned(),
            fields: redacted_fields(block),
        }),
        Some(_) | None => {}
    }
}

fn push_block(parts: &mut Parts, block: &Json, model: Option<&str>) {
    match block {
        Json::Text(text) => parts.text = Some(text.clone()),
        Json::Object(fields) => push_object_block(parts, fields, model),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {}
    }
}

fn concat_text(parts: &[Json]) -> String {
    let mut joined = String::new();
    for part in parts {
        if let Some(text) = part
            .as_object()
            .and_then(|fields| fields.get("text"))
            .and_then(Json::as_str)
        {
            joined.push_str(text);
        }
    }
    joined
}

impl Parts {
    fn message(self) -> Result<Option<Json>, ChatContentError> {
        if self.text.is_none()
            && self.content.is_empty()
            && self.tool_calls.is_empty()
            && self.thinking.is_empty()
        {
            return Ok(None);
        }
        let content = if self.cached_text {
            Json::List(self.content)
        } else if self.content.is_empty() {
            self.text.map_or(Json::Null, Json::Text)
        } else {
            Json::Text(concat_text(&self.content))
        };
        let blocks = if self.thinking.is_empty() {
            Json::Null
        } else {
            let encoded = serde_json::to_value(&self.thinking)?;
            serde_json::from_value(encoded)?
        };
        let reasoning = reasoning_content_from_blocks(&self.thinking);
        let mut message = Object::new();
        message.insert("role".to_owned(), Json::Text("assistant".to_owned()));
        message.insert("content".to_owned(), content);
        message.insert("thinking_blocks".to_owned(), blocks);
        if !self.tool_calls.is_empty() {
            message.insert("tool_calls".to_owned(), Json::List(self.tool_calls));
        }
        if !reasoning.is_empty() {
            message.insert("reasoning_content".to_owned(), Json::Text(reasoning));
        }
        Ok(Some(Json::Object(message)))
    }
}

pub fn push_assistant_message(
    fields: &Object,
    target: &ChatTarget<'_>,
    new_messages: &mut Vec<Json>,
) -> Result<(), ChatContentError> {
    let mut parts = Parts::default();
    match fields.get("content") {
        Some(Json::Text(text)) => parts.text = Some(text.clone()),
        Some(Json::List(blocks)) => {
            for block in blocks {
                push_block(&mut parts, block, target.model);
            }
        }
        Some(_) | None => {}
    }
    if let Some(message) = parts.message()? {
        new_messages.push(message);
    }
    Ok(())
}
