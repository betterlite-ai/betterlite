use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;

#[must_use]
pub fn nested_response(event: &ResponsesAPIStreamingResponse) -> Option<&ResponsesApiResponse> {
    let ResponsesAPIStreamingResponse::Known(known) = event else {
        return None;
    };
    let (KnownResponseEvent::ResponseCreated(record)
    | KnownResponseEvent::ResponseInProgress(record)
    | KnownResponseEvent::ResponseCompleted(record)
    | KnownResponseEvent::ResponseFailed(record)
    | KnownResponseEvent::ResponseIncomplete(record)) = known.as_ref()
    else {
        return None;
    };
    Some(&record.response)
}

#[must_use]
pub fn nested_response_mut(
    event: &mut ResponsesAPIStreamingResponse,
) -> Option<&mut ResponsesApiResponse> {
    let ResponsesAPIStreamingResponse::Known(known) = event else {
        return None;
    };
    let (KnownResponseEvent::ResponseCreated(record)
    | KnownResponseEvent::ResponseInProgress(record)
    | KnownResponseEvent::ResponseCompleted(record)
    | KnownResponseEvent::ResponseFailed(record)
    | KnownResponseEvent::ResponseIncomplete(record)) = known.as_mut()
    else {
        return None;
    };
    Some(&mut record.response)
}

#[must_use]
pub fn completed_response(event: &ResponsesAPIStreamingResponse) -> Option<&ResponsesApiResponse> {
    let ResponsesAPIStreamingResponse::Known(known) = event else {
        return None;
    };
    if let KnownResponseEvent::ResponseCompleted(record) = known.as_ref() {
        Some(&record.response)
    } else {
        None
    }
}
