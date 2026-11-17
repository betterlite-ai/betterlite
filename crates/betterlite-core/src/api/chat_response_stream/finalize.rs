use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::{Extra, ResponseEvent};
use betterlite_openai::api::responses_event_records::records::KnownResponseEvent;
use indexmap::IndexMap;

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
use crate::api::response_id_codecs::response::{build_response_id, is_encoded_response_id};
use crate::api::responses_chat_output::assemble_response;
use crate::api::responses_chat_output::records::{ChatCompletionResponse, RequestFields};

use super::chunk;
use super::error::ChatResponseStreamError;

// The end of the provider stream: the assembled response, then any tool, text and item done events it owes
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
    if let Some(queued) = state.progress.pending_tool_events.drain(..1).next() {
        return Ok(Some(queued.event));
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
    let chat = chat_completion(completion)?;
    let request_text = serde_json::to_string(&state.responses_api_request)?;
    let request: RequestFields = serde_json::from_str(&request_text)
        .map_err(|source| ChatResponseStreamError::Request { source })?;
    let mut response =
        assemble_response(&state.request_input, &request, &chat, state.limits, || {
            (sources.ids)()
        })?
        .response;
    if let Some(raw_id) = state.progress.cached_response_id.clone() {
        response.id = encoded_identity(state, &raw_id)?;
    }
    let aligned = output_items_with_id(
        &response.output,
        "message",
        state.progress.cached_item_id.as_deref(),
    );
    response.output = output_items_with_id(
        &aligned,
        "reasoning",
        state.progress.cached_reasoning_item_id.as_deref(),
    );
    state.progress.completed_response = Some(response.clone());
    Ok(KnownResponseEvent::ResponseCompleted(ResponseEvent {
        response,
        extra: Extra::new(),
    }))
}

// The assembled response is a chat completion object, whose hidden parameters carry the private name
fn chat_completion(completion: &Json) -> Result<ChatCompletionResponse, ChatResponseStreamError> {
    let fields: IndexMap<String, Json> = serde_json::from_str(&serde_json::to_string(completion)?)?;
    let text = serde_json::to_string(&chunk::fold_hidden_params(fields))?;
    serde_json::from_str(&text).map_err(|source| ChatResponseStreamError::Completion { source })
}

// The cached id is encoded the way the created and in-progress events encode theirs
fn encoded_identity<Context>(
    state: &ChatStreamState<Context>,
    raw_id: &str,
) -> Result<String, ChatResponseStreamError> {
    if is_encoded_response_id(raw_id) {
        return Ok(raw_id.to_owned());
    }
    let model_id = match state.litellm_metadata.get("model_info") {
        None | Some(Json::Null) => None,
        Some(Json::Object(fields)) => match fields.get("id") {
            None | Some(Json::Null) => None,
            Some(Json::Text(id)) => Some(id.as_str()),
            Some(_) => {
                return Err(ChatResponseStreamError::Metadata {
                    field: "litellm_metadata.model_info.id",
                });
            }
        },
        Some(_) => {
            return Err(ChatResponseStreamError::Metadata {
                field: "litellm_metadata.model_info",
            });
        }
    };
    Ok(build_response_id(
        state.custom_llm_provider.as_deref(),
        model_id,
        raw_id,
    ))
}
