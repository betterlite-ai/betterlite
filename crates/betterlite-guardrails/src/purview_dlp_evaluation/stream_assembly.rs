use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;

#[must_use]
pub fn assemble_responses_stream(
    chunks: &[ResponsesAPIStreamingResponse],
) -> (bool, Option<ResponsesApiResponse>) {
    let mut looks_like_responses_api = false;
    let mut final_response = None;
    for chunk in chunks {
        match chunk {
            ResponsesAPIStreamingResponse::Generic(event) => {
                looks_like_responses_api |= event.r#type.starts_with("response.");
            }
            ResponsesAPIStreamingResponse::Known(event) => match event.as_ref() {
                KnownResponseEvent::ResponseCreated(inner)
                | KnownResponseEvent::ResponseInProgress(inner)
                | KnownResponseEvent::ResponseCompleted(inner)
                | KnownResponseEvent::ResponseFailed(inner)
                | KnownResponseEvent::ResponseIncomplete(inner) => {
                    looks_like_responses_api = true;
                    final_response = Some(inner.response.clone());
                }
                KnownResponseEvent::ResponsePartAdded(_)
                | KnownResponseEvent::ReasoningSummaryTextDelta(_)
                | KnownResponseEvent::ReasoningSummaryTextDone(_)
                | KnownResponseEvent::ReasoningSummaryPartDone(_)
                | KnownResponseEvent::OutputItemAdded(_)
                | KnownResponseEvent::OutputItemDone(_)
                | KnownResponseEvent::ContentPartAdded(_)
                | KnownResponseEvent::ContentPartDone(_)
                | KnownResponseEvent::OutputTextDelta(_)
                | KnownResponseEvent::OutputTextAnnotationAdded(_)
                | KnownResponseEvent::OutputTextDone(_)
                | KnownResponseEvent::RefusalDelta(_)
                | KnownResponseEvent::RefusalDone(_)
                | KnownResponseEvent::FunctionCallArgumentsDelta(_)
                | KnownResponseEvent::FunctionCallArgumentsDone(_)
                | KnownResponseEvent::FileSearchCallInProgress(_)
                | KnownResponseEvent::FileSearchCallSearching(_)
                | KnownResponseEvent::FileSearchCallCompleted(_)
                | KnownResponseEvent::WebSearchCallInProgress(_)
                | KnownResponseEvent::WebSearchCallSearching(_)
                | KnownResponseEvent::WebSearchCallCompleted(_)
                | KnownResponseEvent::McpListToolsInProgress(_)
                | KnownResponseEvent::McpListToolsCompleted(_)
                | KnownResponseEvent::McpListToolsFailed(_)
                | KnownResponseEvent::McpCallInProgress(_)
                | KnownResponseEvent::McpCallArgumentsDelta(_)
                | KnownResponseEvent::McpCallArgumentsDone(_)
                | KnownResponseEvent::McpCallCompleted(_)
                | KnownResponseEvent::McpCallFailed(_) => looks_like_responses_api = true,
                KnownResponseEvent::Error(_)
                | KnownResponseEvent::ImageGenerationPartialImage(_) => {}
            },
        }
    }
    (looks_like_responses_api, final_response)
}

mod tests;
