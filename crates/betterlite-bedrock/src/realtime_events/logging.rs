use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_core::api::realtime_session_state::DEFAULT_LOGGED_EVENT_TYPES;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::decode::json_str;

#[must_use]
pub fn should_log_event(configured: &LoggedEventTypes, event: &IndexMap<String, Json>) -> bool {
    // An absent type and a type that is not a string both miss every list, exactly as Python's `in` does
    let kind = event.get("type").and_then(json_str);
    match configured {
        LoggedEventTypes::Default => {
            kind.is_some_and(|event_type| DEFAULT_LOGGED_EVENT_TYPES.contains(&event_type))
        }
        LoggedEventTypes::All => true,
        LoggedEventTypes::Only(types) => {
            kind.is_some_and(|event_type| types.iter().any(|logged| logged == event_type))
        }
    }
}
