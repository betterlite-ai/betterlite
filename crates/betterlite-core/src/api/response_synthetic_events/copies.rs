use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use serde::Serialize;
use serde::de::DeserializeOwned;

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
        LoggingEvent::Event(ResponsesAPIStreamingResponse::Generic(generic)) => {
            LoggedCopy::Detached(Box::new(ResponsesAPIStreamingResponse::Generic(
                generic.clone(),
            )))
        }
        LoggingEvent::Event(ResponsesAPIStreamingResponse::Known(known)) => {
            let copied = round_trip(known.as_ref()).unwrap_or_else(|_error| known.as_ref().clone());
            LoggedCopy::Detached(Box::new(ResponsesAPIStreamingResponse::Known(Box::new(
                copied,
            ))))
        }
    }
}

pub(super) fn round_trip<T>(event: &T) -> Result<T, serde_json::Error>
where
    T: Serialize + DeserializeOwned,
{
    let serialized = serde_json::to_value(event)?;
    serde_json::from_value(serialized)
}
