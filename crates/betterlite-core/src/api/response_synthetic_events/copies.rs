use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;

pub enum LoggingEvent<'a> {
    Value(&'a Json),
    Event(&'a ResponsesAPIStreamingResponse),
}

pub enum LoggedCopy<'a> {
    Same(&'a Json),
    Detached(Box<ResponsesAPIStreamingResponse>),
}

#[must_use]
pub fn logging_copy(event: LoggingEvent<'_>) -> LoggedCopy<'_> {
    match event {
        LoggingEvent::Value(value) => LoggedCopy::Same(value),
        LoggingEvent::Event(generic @ ResponsesAPIStreamingResponse::Generic(_)) => {
            LoggedCopy::Detached(Box::new(generic.clone()))
        }
        LoggingEvent::Event(ResponsesAPIStreamingResponse::Known(known)) => {
            let copied = serde_json::to_value(known.as_ref())
                .and_then(serde_json::from_value)
                .unwrap_or_else(|_error| known.as_ref().clone());
            LoggedCopy::Detached(Box::new(ResponsesAPIStreamingResponse::Known(Box::new(
                copied,
            ))))
        }
    }
}
