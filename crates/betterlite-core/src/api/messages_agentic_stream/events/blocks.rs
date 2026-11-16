use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::{event_fields, index_field, object_field, shape_error, text_field};
use crate::api::api_boundaries::ApiLimits;
use crate::api::messages_agentic_stream::{MessagesAgenticError, decode_messages_agentic_stream};

// LiteLLM's _handle_content_block_start: a known type starts from its template, any other is copied
pub(super) fn handle_content_block_start(
    data: &Json,
    blocks: &mut BTreeMap<usize, IndexMap<String, Json>>,
) -> Result<(), MessagesAgenticError> {
    let fields = event_fields(data)?;
    let index = index_field(fields)?.unwrap_or(blocks.len());
    let block = object_field(fields, "content_block")?
        .cloned()
        .unwrap_or_default();
    let entry = match block_kind(&block) {
        BlockKind::ToolUse => block_from(TOOL_USE_TEMPLATE, &block),
        BlockKind::Text => block_from(TEXT_TEMPLATE, &block),
        BlockKind::Thinking => block_from(THINKING_TEMPLATE, &block),
        BlockKind::Redacted => block_from(REDACTED_TEMPLATE, &block),
        BlockKind::Copy => block,
    };
    blocks.insert(index, entry);
    Ok(())
}

enum BlockKind {
    Text,
    Thinking,
    Redacted,
    ToolUse,
    Copy,
}

// LiteLLM's block.get("type", "text") followed by its template lookup; a non-string type copies through
fn block_kind(block: &IndexMap<String, Json>) -> BlockKind {
    match block.get("type") {
        None => BlockKind::Text,
        Some(Json::Text(kind)) => match kind.as_str() {
            "text" => BlockKind::Text,
            "thinking" => BlockKind::Thinking,
            "redacted_thinking" => BlockKind::Redacted,
            "tool_use" => BlockKind::ToolUse,
            _other => BlockKind::Copy,
        },
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            BlockKind::Copy
        }
    }
}

enum Template {
    Literal(&'static str),
    Copied(&'static str),
    Object,
}

const TOOL_USE_TEMPLATE: &[(&str, Template)] = &[
    ("type", Template::Literal("tool_use")),
    ("id", Template::Copied("id")),
    ("name", Template::Copied("name")),
    ("input", Template::Object),
    ("_partial_json", Template::Literal("")),
];
const TEXT_TEMPLATE: &[(&str, Template)] = &[
    ("type", Template::Literal("text")),
    ("text", Template::Literal("")),
];
const THINKING_TEMPLATE: &[(&str, Template)] = &[
    ("type", Template::Literal("thinking")),
    ("thinking", Template::Literal("")),
    ("signature", Template::Literal("")),
];
const REDACTED_TEMPLATE: &[(&str, Template)] = &[
    ("type", Template::Literal("redacted_thinking")),
    ("data", Template::Copied("data")),
];

fn block_from(
    fields: &[(&str, Template)],
    block: &IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    fields
        .iter()
        .map(|(key, template)| ((*key).to_owned(), template_value(template, block)))
        .collect()
}

fn template_value(template: &Template, block: &IndexMap<String, Json>) -> Json {
    match template {
        Template::Literal(text) => Json::Text((*text).to_owned()),
        Template::Copied(key) => block
            .get(*key)
            .cloned()
            .unwrap_or(Json::Text(String::new())),
        Template::Object => Json::Object(IndexMap::new()),
    }
}

// LiteLLM's _handle_content_block_delta: a delta for a block that never started is a no-op
pub(super) fn handle_content_block_delta(
    data: &Json,
    blocks: &mut BTreeMap<usize, IndexMap<String, Json>>,
) -> Result<(), MessagesAgenticError> {
    let (index, delta) = delta_payload(data)?;
    let Some(target) = blocks.get_mut(&index) else {
        return Ok(());
    };
    apply_delta(target, &delta)
}

fn delta_payload(data: &Json) -> Result<(usize, IndexMap<String, Json>), MessagesAgenticError> {
    let fields = event_fields(data)?;
    let index = index_field(fields)?.unwrap_or(0);
    let delta = object_field(fields, "delta")?.cloned().unwrap_or_default();
    Ok((index, delta))
}

fn apply_delta(
    block: &mut IndexMap<String, Json>,
    delta: &IndexMap<String, Json>,
) -> Result<(), MessagesAgenticError> {
    match text_field(delta, "type").unwrap_or_default() {
        "text_delta" => append_text(block, "text", delta, "text"),
        "input_json_delta" => append_text(block, "_partial_json", delta, "partial_json"),
        "thinking_delta" => append_text(block, "thinking", delta, "thinking"),
        "signature_delta" => {
            let signature = signature_value(delta, block);
            block.insert("signature".to_owned(), signature);
            Ok(())
        }
        _other => Ok(()),
    }
}

fn append_text(
    block: &mut IndexMap<String, Json>,
    block_key: &str,
    delta: &IndexMap<String, Json>,
    delta_key: &str,
) -> Result<(), MessagesAgenticError> {
    let current = match block.get(block_key) {
        None => "",
        Some(Json::Text(text)) => text,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(shape_error(block_key, "a string"));
        }
    };
    let appended = match delta.get(delta_key) {
        None => "",
        Some(Json::Text(text)) => text,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(shape_error(delta_key, "a string"));
        }
    };
    let joined = format!("{current}{appended}");
    block.insert(block_key.to_owned(), Json::Text(joined));
    Ok(())
}

fn signature_value(delta: &IndexMap<String, Json>, block: &IndexMap<String, Json>) -> Json {
    delta
        .get("signature")
        .or_else(|| block.get("signature"))
        .cloned()
        .unwrap_or(Json::Text(String::new()))
}

// LiteLLM's _handle_content_block_stop: the accumulated partial JSON becomes the tool input
pub(super) fn handle_content_block_stop(
    data: &Json,
    blocks: &mut BTreeMap<usize, IndexMap<String, Json>>,
    limits: &ApiLimits,
) -> Result<(), MessagesAgenticError> {
    let index = index_field(event_fields(data)?)?.unwrap_or(0);
    if let Some(target) = blocks.get_mut(&index) {
        finish_tool_input(target, limits)?;
    }
    Ok(())
}

fn finish_tool_input(
    block: &mut IndexMap<String, Json>,
    limits: &ApiLimits,
) -> Result<(), MessagesAgenticError> {
    if text_field(block, "type") != Some("tool_use") {
        return Ok(());
    }
    match block.shift_remove("_partial_json") {
        None => Ok(()),
        Some(Json::Text(partial)) => apply_partial(block, &partial, limits),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(shape_error("_partial_json", "a string"))
        }
    }
}

fn apply_partial(
    block: &mut IndexMap<String, Json>,
    partial: &str,
    limits: &ApiLimits,
) -> Result<(), MessagesAgenticError> {
    if partial.is_empty() {
        return Ok(());
    }
    let input = match decode_messages_agentic_stream(partial.as_bytes(), limits) {
        Ok(input) => input,
        // LiteLLM keeps an unparsable partial verbatim so the hook still sees the tool call
        Err(MessagesAgenticError::Json(_source)) => Json::Object(raw_input(partial.to_owned())),
        Err(error) => return Err(error),
    };
    block.insert("input".to_owned(), input);
    Ok(())
}

fn raw_input(partial: String) -> IndexMap<String, Json> {
    let mut input = IndexMap::new();
    input.insert("_raw".to_owned(), Json::Text(partial));
    input
}
