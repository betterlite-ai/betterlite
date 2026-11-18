use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;

// the nested response a lifecycle event carries; a generic event has none, so nothing is typed
#[must_use]
pub fn nested_response(event: &ResponsesAPIStreamingResponse) -> Option<&ResponsesApiResponse> {
    let ResponsesAPIStreamingResponse::Known(known) = event else {
        return None;
    };
    match known.as_ref() {
        KnownResponseEvent::ResponseCreated(event)
        | KnownResponseEvent::ResponseInProgress(event)
        | KnownResponseEvent::ResponseCompleted(event)
        | KnownResponseEvent::ResponseFailed(event)
        | KnownResponseEvent::ResponseIncomplete(event) => Some(&event.response),
        _ => None,
    }
}

#[must_use]
pub fn nested_response_mut(
    event: &mut ResponsesAPIStreamingResponse,
) -> Option<&mut ResponsesApiResponse> {
    let ResponsesAPIStreamingResponse::Known(known) = event else {
        return None;
    };
    match known.as_mut() {
        KnownResponseEvent::ResponseCreated(event)
        | KnownResponseEvent::ResponseInProgress(event)
        | KnownResponseEvent::ResponseCompleted(event)
        | KnownResponseEvent::ResponseFailed(event)
        | KnownResponseEvent::ResponseIncomplete(event) => Some(&mut event.response),
        _ => None,
    }
}

// _get_completed_response_object and the terminal type check: only response.completed is cacheable
#[must_use]
pub fn completed_response(event: &ResponsesAPIStreamingResponse) -> Option<&ResponsesApiResponse> {
    let ResponsesAPIStreamingResponse::Known(known) = event else {
        return None;
    };
    match known.as_ref() {
        KnownResponseEvent::ResponseCompleted(event) => Some(&event.response),
        _ => None,
    }
}
