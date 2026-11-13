use std::collections::{BTreeMap, BTreeSet};

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::KnownResponseEvent;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use indexmap::IndexMap;

use super::error::StreamToolsError;
use super::fields::object;
use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_codex_tools::custom_tools::extract_custom_tool_names;
use crate::api::responses_tool_namespaces::namespace_names::{NamespacedName, name_map};

pub const FIRST_TOOL_OUTPUT_INDEX: i64 = 1;

#[derive(Debug, Clone)]
pub struct QueuedToolEvent {
    pub output_index: i64,
    pub event: KnownResponseEvent,
}

// Everything LiteLLM's iterator changes while it reads chunks, kept apart from the request it was built from
#[derive(Debug, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is one independent LiteLLM iterator event latch, never a bit in a shared mode"
)]
pub struct StreamProgress {
    pub chunk_snapshots: Vec<IndexMap<String, Json>>,
    pub finished: bool,
    pub sent_response_created: bool,
    pub sent_in_progress: bool,
    pub sent_output_item_added: bool,
    pub sent_message_item_added: bool,
    pub sent_content_part_added: bool,
    pub sent_output_text_done: bool,
    pub sent_content_part_done: bool,
    pub sent_output_item_done: bool,
    pub sent_annotation_events: bool,
    pub litellm_model_response: Option<Json>,
    pub completed_response: Option<ResponsesApiResponse>,
    pub final_text: String,
    pub cached_item_id: Option<String>,
    pub message_output_index: i64,
    pub cached_response_id: Option<String>,
    pub buffered_chunk: Option<Json>,
    pub upstream_exhausted: bool,
    pub response_id_primed: bool,
    pub pending_tool_events: Vec<QueuedToolEvent>,
    pub tool_output_index_by_call_id: BTreeMap<String, i64>,
    pub tool_args_by_call_id: BTreeMap<String, String>,
    pub tool_item_id_by_call_id: BTreeMap<String, String>,
    pub tool_call_id_by_index: BTreeMap<i64, String>,
    pub ambiguous_tool_call_indexes: BTreeSet<i64>,
    pub next_tool_output_index: i64,
    pub final_tool_events_queued: bool,
    pub sequence_number: i64,
    pub cached_reasoning_item_id: Option<String>,
    pub sent_reasoning_text_done: bool,
    pub sent_reasoning_part_done: bool,
    pub reasoning_summary_text: String,
    pub pending_response_events: Vec<KnownResponseEvent>,
    pub reasoning_active: bool,
    pub reasoning_done_emitted: bool,
    pub reasoning_item_id: Option<String>,
    pub reasoning_content_parts: Vec<String>,
    pub provider_specific_fields: IndexMap<String, Json>,
    pub web_search_calls: BTreeMap<String, Json>,
    pub queued_web_search_call_ids: BTreeSet<String>,
}

#[derive(Debug)]
pub struct ChatStreamState<Context = LoggingState> {
    pub model: String,
    pub logging_context: Context,
    pub request_input: Json,
    pub responses_api_request: Json,
    pub custom_llm_provider: Option<String>,
    pub litellm_metadata: IndexMap<String, Json>,
    pub custom_tool_names: BTreeSet<String>,
    pub namespace_tool_names: BTreeMap<String, NamespacedName>,
    pub limits: ApiLimits,
    pub progress: StreamProgress,
}

pub struct StreamSetup<'setup, Context> {
    pub model: &'setup str,
    pub logging_context: Context,
    pub request_input: &'setup Json,
    pub responses_api_request: &'setup Json,
    pub custom_llm_provider: Option<&'setup str>,
    pub litellm_metadata: Option<IndexMap<String, Json>>,
    pub limits: ApiLimits,
}

impl<Context> ChatStreamState<Context> {
    pub fn new(setup: StreamSetup<'_, Context>) -> Result<Self, StreamToolsError> {
        let tools = request_tools(setup.responses_api_request)?;
        let mut state = Self {
            model: setup.model.to_owned(),
            logging_context: setup.logging_context,
            request_input: setup.request_input.clone(),
            responses_api_request: setup.responses_api_request.clone(),
            custom_llm_provider: setup.custom_llm_provider.map(str::to_owned),
            litellm_metadata: setup.litellm_metadata.unwrap_or_default(),
            custom_tool_names: extract_custom_tool_names(tools),
            namespace_tool_names: name_map(tools)?,
            limits: setup.limits,
            progress: StreamProgress::default(),
        };
        // output index 0 is reserved for the message item, so the first tool call takes 1
        state.progress.next_tool_output_index = FIRST_TOOL_OUTPUT_INDEX;
        Ok(state)
    }
}

pub fn next_sequence<Context>(
    state: &mut ChatStreamState<Context>,
) -> Result<i64, StreamToolsError> {
    let next = state
        .progress
        .sequence_number
        .checked_add(1)
        .ok_or(StreamToolsError::SequenceOverflow)?;
    state.progress.sequence_number = next;
    Ok(next)
}

pub fn assign_tool_output_index<Context>(
    state: &mut ChatStreamState<Context>,
    call_id: &str,
) -> Result<i64, StreamToolsError> {
    if let Some(index) = state.progress.tool_output_index_by_call_id.get(call_id) {
        return Ok(*index);
    }
    let index = state.progress.next_tool_output_index;
    state.progress.next_tool_output_index = index
        .checked_add(1)
        .ok_or(StreamToolsError::IndexOverflow)?;
    state
        .progress
        .tool_output_index_by_call_id
        .insert(call_id.to_owned(), index);
    Ok(index)
}

fn request_tools(request: &Json) -> Result<&[Json], StreamToolsError> {
    match object(request).and_then(|fields| fields.get("tools")) {
        None | Some(Json::Null) => Ok(&[]),
        Some(Json::List(tools)) => Ok(tools),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(StreamToolsError::ToolsNotList)
        }
    }
}
