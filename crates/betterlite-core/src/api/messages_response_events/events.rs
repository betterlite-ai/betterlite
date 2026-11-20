use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::sdk::unions::SdkOutput;
use betterlite_prompts::reasoning_fields::encrypted_signature::encrypted_reasoning_signature;

use crate::api::messages_chat_output::output::AnthropicStopReason;
use crate::api::messages_options::refusal::{OutputItem, responses_output_refusal_text};
use crate::api::messages_responses_input::usage::anthropic_usage;

use super::MessageStreamState;
use super::SUMMARY_PART_SEPARATOR;
use super::StreamFlag;
use super::chunks;
use super::error::MessagesResponseEventError;
use super::input::{ItemObject, ParsedEvent, is_present, text};

const NO_OUTPUT: &[OutputItem<'static>] = &[];

// The two names a content_block_delta carries: the delta's own type and the field holding its payload.
type DeltaNames = (&'static str, &'static str);

const TEXT_DELTA: DeltaNames = ("text_delta", "text");
const THINKING_DELTA: DeltaNames = ("thinking_delta", "thinking");
const INPUT_JSON_DELTA: DeltaNames = ("input_json_delta", "partial_json");
const SIGNATURE_DELTA: DeltaNames = ("signature_delta", "signature");

// A truthy field of an output item: getattr and dict.get both answer None when the key is absent.
fn item_field<'item>(item: ItemObject<'item>, key: &str) -> Option<&'item Json> {
    item.fields
        .and_then(|fields| fields.get(key))
        .filter(|value| is_present(value))
}

// The handlers of _process_event; they are free functions so MessageStreamState keeps its one inherent impl.
pub(super) fn item_added(state: &mut MessageStreamState, event: ParsedEvent<'_>) {
    let Some(item) = event.item() else {
        return;
    };
    let item_id = item_field(item, "id");
    let Some(item_type) = item_field(item, "type").and_then(text) else {
        return;
    };
    if item_type == "function_call" {
        open_function_call(state, item, item_id);
        return;
    }
    if item_type != "message" {
        return;
    }
    state.open_block(
        item_id,
        chunks::typed_object("text", vec![("text", Json::Text(String::new()))]),
    );
}

fn open_function_call(
    state: &mut MessageStreamState,
    item: ItemObject<'_>,
    item_id: Option<&Json>,
) {
    let call_id = item_field(item, "call_id")
        .cloned()
        .unwrap_or(Json::Text(String::new()));
    let name = item_field(item, "name")
        .cloned()
        .unwrap_or(Json::Text(String::new()));
    if let Some(Json::Text(text_id)) = item_id {
        state
            .pending_tool_ids
            .insert(text_id.clone(), call_id.clone());
    }
    state.open_block(item_id, chunks::tool_block(&call_id, &name));
}

// The block a text delta lands in: the mapped block, or a fresh text block opened for it.
fn text_target(state: &mut MessageStreamState, item_id: Option<&Json>) -> i64 {
    let block_idx = state.block_index_for(item_id);
    if block_idx >= 0 {
        return block_idx;
    }
    state.open_block(
        item_id,
        chunks::typed_object("text", vec![("text", Json::Text(String::new()))]),
    )
}

pub(super) fn refusal_delta(state: &mut MessageStreamState, event: ParsedEvent<'_>) {
    let Some(delta) = text(event.delta()) else {
        return;
    };
    if delta.is_empty() {
        return;
    }
    state.refusal_text.push_str(delta);
    emit_delta(state, event, DeltaKind::Text);
}

// The delta branches of _process_event differ only in the block they pick and the payload they carry.
#[derive(Clone, Copy)]
pub(super) enum DeltaKind {
    Text,
    SummaryPart,
    SummaryText,
    Arguments,
}

pub(super) fn emit_delta(state: &mut MessageStreamState, event: ParsedEvent<'_>, kind: DeltaKind) {
    match kind {
        DeltaKind::Text => {
            let block_idx = text_target(state, event.item_id());
            push_delta(state, block_idx, TEXT_DELTA, event.delta().clone());
        }
        DeltaKind::SummaryPart => {
            let block_idx = state.mapped_block_index(event.field("item_id"));
            if block_idx < 0 || !qualifies_as_summary_index(event.field("summary_index")) {
                return;
            }
            let payload = Json::Text(SUMMARY_PART_SEPARATOR.to_owned());
            push_delta(state, block_idx, THINKING_DELTA, payload);
        }
        DeltaKind::SummaryText => {
            let Some(block_idx) = summary_text_target(state, event) else {
                return;
            };
            let payload = event.delta().clone();
            push_delta(state, block_idx, THINKING_DELTA, payload);
        }
        DeltaKind::Arguments => {
            let block_idx = state.arguments_block_index(event.item_id());
            let payload = event.delta().clone();
            push_delta(state, block_idx, INPUT_JSON_DELTA, payload);
        }
    }
}

// A summary text delta opens its thinking block only when it carries a delta.
fn summary_text_target(state: &mut MessageStreamState, event: ParsedEvent<'_>) -> Option<i64> {
    let item_id = event.item_id();
    let block_idx = state.block_index_for(item_id);
    if block_idx >= 0 {
        return Some(block_idx);
    }
    if is_present(event.delta()) {
        return Some(state.open_block(item_id, chunks::thinking_block()));
    }
    None
}

pub(super) fn item_done(state: &mut MessageStreamState, event: ParsedEvent<'_>) {
    let item = event.item();
    let item_id = item.and_then(|present| item_field(present, "id"));
    let is_reasoning = item
        .and_then(|present| item_field(present, "type"))
        .and_then(text)
        == Some("reasoning");
    if is_reasoning {
        close_reasoning_item(state, item, item_id);
        return;
    }
    let block_idx = state.block_index_for(item_id);
    if block_idx < 0 {
        return;
    }
    state
        .chunk_queue
        .push_back(chunks::content_block_stop(block_idx));
}

fn close_reasoning_item(
    state: &mut MessageStreamState,
    item: Option<ItemObject<'_>>,
    item_id: Option<&Json>,
) {
    let block_idx = state.block_index_for(item_id);
    let signature = item
        .and_then(|present| item_field(present, "encrypted_content"))
        .and_then(text)
        .map(encrypted_reasoning_signature);
    if block_idx < 0 && signature.is_none() {
        return;
    }
    if block_idx < 0 {
        let data = signature.unwrap_or_default();
        let redacted_idx = state.open_block(item_id, chunks::redacted_thinking_block(&data));
        state
            .chunk_queue
            .push_back(chunks::content_block_stop(redacted_idx));
        return;
    }
    if let Some(encoded) = signature {
        push_delta(state, block_idx, SIGNATURE_DELTA, Json::Text(encoded));
    }
    state
        .chunk_queue
        .push_back(chunks::content_block_stop(block_idx));
}

pub(super) fn completed(
    state: &mut MessageStreamState,
    event: ParsedEvent<'_>,
) -> Result<(), MessagesResponseEventError> {
    let (output, status, usage) = event
        .response()
        .map_or((NO_OUTPUT, None, None), |attributes| {
            (attributes.output, attributes.status, attributes.usage)
        });
    let refusal_text = final_refusal_text(output, &state.refusal_text);
    let stop_reason = stop_reason(status, refusal_text.as_deref(), output);
    let projected_usage = anthropic_usage(usage).map_err(MessagesResponseEventError::Usage)?;
    let stop_details = refusal_text
        .as_deref()
        .filter(|_| stop_reason == AnthropicStopReason::Refusal);
    let delta = chunks::message_delta(stop_reason, stop_details, &projected_usage)?;
    state.chunk_queue.push_back(delta);
    state
        .chunk_queue
        .push_back(chunks::typed_object("message_stop", Vec::new()));
    state.sent_message_stop = StreamFlag(true);
    Ok(())
}

// A content_block_delta carries one typed delta: its two names and the single payload it holds.
fn push_delta(state: &mut MessageStreamState, block_idx: i64, names: DeltaNames, payload: Json) {
    let delta = chunks::typed_object(names.0, vec![(names.1, payload)]);
    state.chunk_queue.push_back(chunks::block_frame(
        "content_block_delta",
        block_idx,
        "delta",
        delta,
    ));
}

fn final_refusal_text(output: &[OutputItem<'_>], streamed: &str) -> Option<String> {
    match responses_output_refusal_text(output) {
        Some(text) => Some(text.to_owned()),
        None if streamed.is_empty() => None,
        None => Some(streamed.to_owned()),
    }
}

fn stop_reason(
    status: Option<&str>,
    refusal_text: Option<&str>,
    output: &[OutputItem<'_>],
) -> AnthropicStopReason {
    if status == Some("incomplete") {
        return AnthropicStopReason::MaxTokens;
    }
    if refusal_text.is_some() {
        return AnthropicStopReason::Refusal;
    }
    if output.iter().any(is_function_call) {
        return AnthropicStopReason::ToolUse;
    }
    AnthropicStopReason::EndTurn
}

fn is_function_call(item: &OutputItem<'_>) -> bool {
    match item {
        OutputItem::Typed(SdkOutput::FunctionCall(_)) => true,
        OutputItem::Typed(_) => false,
        OutputItem::Raw(raw) => matches!(
            raw,
            Json::Object(fields)
                if matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "function_call")
        ),
    }
}

// isinstance(summary_index, int) includes bools; a float or a non-integer string never qualifies.
fn qualifies_as_summary_index(value: Option<&Json>) -> bool {
    match value {
        Some(Json::Bool(flag)) => *flag,
        Some(Json::Number(number)) => {
            let text = number.to_string();
            !text.contains(['.', 'e', 'E']) && text != "0" && text != "-0"
        }
        Some(Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_)) | None => false,
    }
}
