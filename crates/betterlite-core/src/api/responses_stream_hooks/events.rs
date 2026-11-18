use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;

// Both borrow modes project the same lifecycle variants without copying the response.
macro_rules! response_access {
    ($name:ident $(, $mutable:tt)?) => {
        #[must_use]
        pub fn $name(event: & $($mutable)? ResponsesAPIStreamingResponse) -> Option<& $($mutable)? ResponsesApiResponse> {
            let ResponsesAPIStreamingResponse::Known(known) = event else { return None; };
            let (KnownResponseEvent::ResponseCreated(record)
                | KnownResponseEvent::ResponseInProgress(record)
                | KnownResponseEvent::ResponseCompleted(record)
                | KnownResponseEvent::ResponseFailed(record)
                | KnownResponseEvent::ResponseIncomplete(record)) = & $($mutable)? **known else { return None; };
            Some(& $($mutable)? record.response)
        }
    };
}

response_access!(nested_response);
response_access!(nested_response_mut, mut);
