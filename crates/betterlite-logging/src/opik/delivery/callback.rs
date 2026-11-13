use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;

// The litellm callback arguments OpikLogger reads: kwargs, response_obj, start_time and end_time.
#[derive(Clone, Copy, Debug)]
pub struct Call<'event> {
    pub kwargs: &'event IndexMap<String, Json>,
    pub response: &'event IndexMap<String, Json>,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
}

pub(crate) fn should_skip_event(kwargs: &IndexMap<String, Json>) -> bool {
    match kwargs.get("standard_logging_object") {
        None | Some(Json::Null) => {
            tracing::debug!("OpikLogger skipping event; no standard_logging_object found");
            true
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            false
        }
    }
}
