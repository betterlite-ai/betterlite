use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::{Extra, ResponseEvent};
use betterlite_openai::api::responses_event_records::records::KnownResponseEvent;

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
use crate::api::response_id_traversal::response_id::{
    ResponseRewriteError, rewrite_typed_response, transcode,
};
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
    // A null metadata value reads as the field's default, as docs/compatibility.md says for a typed field
    let mut metadata = state.litellm_metadata.clone();
    metadata.retain(|_name, value| !matches!(value, Json::Null));
    rewrite_typed_response(
        &mut response,
        state.custom_llm_provider.as_deref(),
        &transcode(&metadata)?,
    )?;
    state.progress.completed_response = Some(response.clone());
    Ok(KnownResponseEvent::ResponseCompleted(ResponseEvent {
        response,
        extra: Extra::new(),
    }))
}

impl From<ResponseRewriteError> for ChatResponseStreamError {
    fn from(error: ResponseRewriteError) -> Self {
        match error {
            ResponseRewriteError::Encode(source) => {
                Self::Items(StreamItemsError::Encode { source })
            }
            ResponseRewriteError::Decode(source) => {
                Self::Items(StreamItemsError::Response { source })
            }
            ResponseRewriteError::Traversal(source) => Self::Traversal(source),
        }
    }
}
