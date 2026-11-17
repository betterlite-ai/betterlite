use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::{Extra, ResponseEvent};
use betterlite_openai::api::responses_event_records::records::KnownResponseEvent;
use betterlite_openai::api::responses_objects::ResponseOutputItem;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::api::chat_response_stream_items::error::StreamItemsError;
use crate::api::chat_response_stream_items::progression::{
    is_stream_finished, return_default_done_events,
};
use crate::api::chat_response_stream_items::snapshot::{
    StreamAssembly, create_litellm_model_response,
};
use crate::api::chat_response_stream_items::sources::EventSources;
use crate::api::chat_response_stream_tools::final_tools::queue_final_tool_completion;
use crate::api::chat_response_stream_tools::items::output_items_with_id;
use crate::api::chat_response_stream_tools::state::ChatStreamState;
use crate::api::response_id_traversal::record::{DictRecord, Record};
use crate::api::response_id_traversal::response_id::rewrite_response_id;
use crate::api::responses_chat_output::assemble_response;
use crate::api::responses_chat_output::records::{ChatCompletionResponse, RequestFields};

use super::chunk;
use super::error::ChatResponseStreamError;

pub(crate) fn common_done_event<Context>(
    state: &mut ChatStreamState<Context>,
    sources: &EventSources,
) -> Result<Option<KnownResponseEvent>, ChatResponseStreamError>
where
    Context: StreamAssembly,
{
    if state.progress.litellm_model_response.is_none() {
        state.progress.litellm_model_response = create_litellm_model_response(state)?;
    }
    let Some(completion) = state.progress.litellm_model_response.clone() else {
        return Ok(None);
    };
    queue_final_tool_completion(state, &completion)?;
    if !state.progress.pending_tool_events.is_empty() {
        return Ok(Some(state.progress.pending_tool_events.remove(0).event));
    }
    if let Some(done) = return_default_done_events(state, sources, &completion)? {
        return Ok(Some(done));
    }
    state.progress.finished = is_stream_finished(state);
    completed_event(state, sources, &completion).map(Some)
}

// response.completed keeps the same response event type when the finish reason makes its status incomplete
fn completed_event<Context>(
    state: &mut ChatStreamState<Context>,
    sources: &EventSources,
    completion: &Json,
) -> Result<KnownResponseEvent, ChatResponseStreamError> {
    let fields = serde_json::from_str(&serde_json::to_string(completion)?)?;
    let text = serde_json::to_string(&chunk::fold_hidden_params(fields))?;
    let chat: ChatCompletionResponse = serde_json::from_str(&text)
        .map_err(|source| ChatResponseStreamError::Completion { source })?;
    let request_text = serde_json::to_string(&state.responses_api_request)?;
    let request: RequestFields = serde_json::from_str(&request_text)
        .map_err(|source| ChatResponseStreamError::Request { source })?;
    let mut response =
        assemble_response(&state.request_input, &request, &chat, state.limits, || {
            (sources.ids)()
        })?
        .response;
    if let Some(raw_id) = state.progress.cached_response_id.clone() {
        response.id = raw_id;
    }
    let message_id = state.progress.cached_item_id.as_deref();
    let reasoning_id = state.progress.cached_reasoning_item_id.as_deref();
    let aligned = output_items_with_id(&response.output, "message", message_id);
    response.output = output_items_with_id(&aligned, "reasoning", reasoning_id);
    // The completed event takes the managed-id rewrite a non-streaming response gets: id, encrypted items, containers
    let mut record = json_record(transcode(&response)?);
    // A null metadata value reads as the field's default, as docs/compatibility.md says for a typed field
    let mut metadata = state.litellm_metadata.clone();
    metadata.retain(|_name, value| !matches!(value, Json::Null));
    rewrite_response_id(
        &mut record,
        state.custom_llm_provider.as_deref(),
        &transcode(&metadata)?,
    )?;
    let rewritten: RewrittenFields = transcode(&record)?;
    response.id = rewritten.id;
    response.output = rewritten
        .output
        .into_iter()
        .map(ResponseOutputItem::Unknown)
        .collect();
    state.progress.completed_response = Some(response.clone());
    Ok(KnownResponseEvent::ResponseCompleted(ResponseEvent {
        response,
        extra: Extra::new(),
    }))
}

// B904's rewrite changes only the id and the output items, which stay raw so a typed re-parse adds no fields
#[derive(Deserialize)]
struct RewrittenFields {
    id: String,
    output: Vec<BTreeMap<String, Json>>,
}

// serde's untagged Record decode turns a float into its private number map, so JSON converts by hand
fn json_record(value: Json) -> Record {
    match value {
        Json::Object(fields) => Record::Dict(DictRecord {
            values: fields
                .into_iter()
                .map(|(name, child)| (name, json_record(child)))
                .collect(),
        }),
        Json::List(items) => Record::List(items.into_iter().map(json_record).collect()),
        scalar @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_)) => {
            Record::Scalar(scalar)
        }
    }
}

// Every conversion here crosses JSON, so one serde pass each way and a failure names its field
fn transcode<Source, Target>(source: &Source) -> Result<Target, StreamItemsError>
where
    Source: Serialize + ?Sized,
    Target: DeserializeOwned,
{
    let encoded = serde_json::to_vec(source)?;
    let mut decoder = serde_json::Deserializer::from_slice(&encoded);
    serde_path_to_error::deserialize(&mut decoder).map_err(StreamItemsError::from)
}
