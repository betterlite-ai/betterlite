use betterlite_models::decisions::Json;

use super::content_part::{ContentPartCache, normalize_event};
use super::error::XaiRealtimeError;
use super::index_sets::inject_missing_indices;
use super::role::repair_tool_role;
use super::usage::normalize_response_usage;

#[derive(Debug, Default)]
pub struct XaiRealtimeNormalizer {
    parts: ContentPartCache,
}

#[must_use]
pub fn should_drop(event: &Json) -> bool {
    matches!(
        event,
        Json::Object(fields)
            if matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "ping")
    )
}

fn event_type(event: &Json) -> Option<&str> {
    match event {
        Json::Object(fields) => match fields.get("type") {
            Some(Json::Text(kind)) => Some(kind),
            _ => None,
        },
        _ => None,
    }
}

impl XaiRealtimeNormalizer {
    pub fn normalize(&mut self, event: Json) -> Result<Json, XaiRealtimeError> {
        let event_type = event_type(&event).map(str::to_owned);
        let event = normalize_event(event, event_type.as_deref(), &mut self.parts)?;
        let event = repair_tool_role(event, event_type.as_deref());
        let event = inject_missing_indices(event, event_type.as_deref());
        Ok(normalize_response_usage(event, event_type.as_deref()))
    }
}
