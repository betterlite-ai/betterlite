use betterlite_anthropic::message_replay::is_empty_unsigned_thinking;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_models::decisions::Json as ModelJson;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize as _;
use serde::de::value::{Error as ValueError, StrDeserializer};
use serde_json::Number;

use crate::api::messages_chat_output::error::ChatOutputError;
use crate::api::messages_chat_output::records::{ChoiceRecord, DeltaRecord};
use crate::api::messages_chat_output::stream::content_block;
use crate::api::messages_options::refusal::{ChatTurn, chat_refusal_text, refusal_stop_details};
use crate::api::messages_stream_split::delta::StreamingDeltaType;

use super::error::StreamStateError;
use super::events::{block_kind, json_from_serialize, start_object};
use super::state::{BlockKind, StreamState};

// B963 reads refusals from the models Json vocabulary while B966's records carry the prompts one
fn refusal_text(delta: &DeltaRecord) -> Option<String> {
    let refusal = if delta.refusal.is_empty() {
        ModelJson::Null
    } else {
        ModelJson::Text(delta.refusal.clone())
    };
    let fields = ModelJson::from(&Json::Object(delta.provider_specific_fields.clone()));
    chat_refusal_text(&ChatTurn {
        refusal: &refusal,
        provider_specific_fields: &fields,
    })
    .map(ToOwned::to_owned)
}

// The declared delta type names the one payload key its content travels in, and a closed set has no others
fn payload_field(delta: &Object) -> Option<&'static str> {
    let name = delta.get("type").and_then(Json::as_str)?;
    let kind = StreamingDeltaType::deserialize(StrDeserializer::<ValueError>::new(name)).ok()?;
    Some(kind.payload_field())
}

// An empty delta carries nothing, and a delta type outside the translate layer's set is treated as empty
#[must_use]
pub fn delta_has_content(event: &Object) -> bool {
    if event.get("type").and_then(Json::as_str) != Some("content_block_delta") {
        return false;
    }
    let Some(Json::Object(delta)) = event.get("delta") else {
        return false;
    };
    let Some(field) = payload_field(delta) else {
        return false;
    };
    delta.get(field).is_some_and(Json::is_truthy)
}

// A chunk that only repeats its block type opens nothing: an empty thinking block is rejected as history
pub fn is_blank_delta(choices: &[ChoiceRecord]) -> Result<bool, StreamStateError> {
    let Some(choice) = choices.first() else {
        return Err(StreamStateError::from(ChatOutputError::NoChoices));
    };
    if choice.finish_reason.is_some() {
        return Ok(false);
    }
    let delta = &choice.delta;
    if !delta.tool_calls.is_empty() {
        return Ok(false);
    }
    if delta
        .content
        .as_deref()
        .is_some_and(|text| !text.is_empty())
    {
        return Ok(false);
    }
    if refusal_text(delta).is_some() {
        return Ok(false);
    }
    if !delta.reasoning_content.is_empty() {
        return Ok(false);
    }
    let opens_thinking = delta
        .thinking_blocks
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .any(|block| !is_empty_unsigned_thinking(&Json::Object(block.clone())));
    Ok(!opens_thinking)
}

// Every compaction frame names its type and the index the synthetic block occupies
fn compaction_event(kind: &str, index: i64) -> Object {
    let mut event = Object::new();
    event.insert("type".to_owned(), Json::Text(kind.to_owned()));
    event.insert("index".to_owned(), Json::Number(Number::from(index)));
    event
}

// The block mirrors the text-block shape, since clients introspect content_block_start for the full schema
fn compaction_start(index: i64) -> Object {
    let mut block = Object::new();
    block.insert("type".to_owned(), Json::Text("compaction".to_owned()));
    block.insert("content".to_owned(), Json::Text(String::new()));
    let mut event = compaction_event("content_block_start", index);
    event.insert("content_block".to_owned(), Json::Object(block));
    event
}

fn compaction_delta_event(index: i64, summary: &str) -> Object {
    let mut delta = Object::new();
    delta.insert("type".to_owned(), Json::Text("compaction_delta".to_owned()));
    delta.insert("content".to_owned(), Json::Text(summary.to_owned()));
    let mut event = compaction_event("content_block_delta", index);
    event.insert("delta".to_owned(), Json::Object(delta));
    event
}

// A type change opens a block, and so does a new named tool call while a tool block stays open
pub fn should_start_new_content_block<S>(
    state: &mut StreamState<S>,
    choices: &[ChoiceRecord],
    ids: &dyn IdSource,
) -> Result<bool, StreamStateError> {
    let Some(choice) = choices.first() else {
        return Err(StreamStateError::from(ChatOutputError::NoChoices));
    };
    if choice.finish_reason.is_some() {
        return Ok(false);
    }
    if let Some(text) = refusal_text(&choice.delta) {
        state.refusal_text.push_str(&text);
    }
    let block = content_block(choices, ids)?;
    let kind = block_kind(&block);
    let mut start = start_object(&block)?;
    if kind == BlockKind::ToolUse
        && let Some(Json::Text(name)) = start.get("name")
        && !name.is_empty()
    {
        let restored = state
            .tool_name_mapping
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.clone());
        start.insert("name".to_owned(), Json::Text(restored));
    }
    if kind != state.current_content_block_type {
        state.current_content_block_type = kind;
        state.current_content_block_start = Json::Object(start);
        return Ok(true);
    }
    if kind == BlockKind::ToolUse && start.get("name").is_some_and(Json::is_truthy) {
        state.current_content_block_start = Json::Object(start);
        return Ok(true);
    }
    Ok(false)
}

// A refusal is the stop reason unless the provider already ran out of tokens
pub fn with_refusal_stop_details<S>(
    state: &StreamState<S>,
    mut event: Object,
) -> Result<Object, StreamStateError> {
    let Some(kind) = event.get("type") else {
        return Err(StreamStateError::MalformedEvent { field: "type" });
    };
    if kind.as_str() != Some("message_delta") || state.refusal_text.is_empty() {
        return Ok(event);
    }
    let Some(mut delta) = event.get("delta").and_then(Json::as_object).cloned() else {
        return Err(StreamStateError::MalformedEvent { field: "delta" });
    };
    if delta.get("stop_reason").and_then(Json::as_str) == Some("max_tokens") {
        return Ok(event);
    }
    delta.insert("stop_reason".to_owned(), Json::Text("refusal".to_owned()));
    delta.insert(
        "stop_details".to_owned(),
        json_from_serialize(&refusal_stop_details(Some(state.refusal_text.clone())))?,
    );
    event.insert("delta".to_owned(), Json::Object(delta));
    Ok(event)
}

// One event per call, so the index advances only once the terminal stop is handed back
pub fn next_compaction_event<S>(state: &mut StreamState<S>) -> Option<Object> {
    if state.compaction_block.is_none() || state.sent_compaction_block {
        return None;
    }
    let index = state.current_content_block_index;
    if !state.compaction_progress.sent_compaction_block_start {
        state.compaction_progress.sent_compaction_block_start = true;
        return Some(compaction_start(index));
    }
    if !state.compaction_progress.sent_compaction_block_delta {
        state.compaction_progress.sent_compaction_block_delta = true;
        let summary = state
            .compaction_block
            .as_ref()
            .and_then(|block| block.content.clone())
            .unwrap_or_default();
        return Some(compaction_delta_event(index, &summary));
    }
    state.current_content_block_index = state.current_content_block_index.saturating_add(1);
    state.sent_compaction_block = true;
    Some(compaction_event("content_block_stop", index))
}
