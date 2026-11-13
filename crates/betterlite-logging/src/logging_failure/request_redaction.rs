use betterlite_config::sections::logging_privacy::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::logging_privacy::PrivacyContext;
use crate::logging_privacy::request::decide_request;
use crate::logging_state::LoggingState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestRedaction {
    LoggerDecides,
    Redact,
}

pub fn request_redacts_messages(state: &LoggingState, settings: Settings) -> RequestRedaction {
    let mut request = IndexMap::new();
    if let Some(flag) = state
        .callback_dynamic_params
        .get("turn_off_message_logging")
    {
        request.insert("turn_off_message_logging".to_owned(), flag.clone());
    }
    for slot in ["litellm_metadata", "metadata"] {
        if let Some(Json::Object(metadata)) = state.litellm_params.get(slot)
            && let Some(headers) = metadata.get("headers")
        {
            let only_headers = IndexMap::from([("headers".to_owned(), headers.clone())]);
            request.insert(slot.to_owned(), Json::Object(only_headers));
        }
    }
    match decide_request(&request, settings, PrivacyContext::default(), true) {
        Ok(decision) if !decision.content => RequestRedaction::LoggerDecides,
        Ok(_) => RequestRedaction::Redact,
        Err(error) => {
            tracing::warn!(%error, "the request's redaction settings are unreadable, so its messages are redacted");
            RequestRedaction::Redact
        }
    }
}
