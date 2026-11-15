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
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => None,
        },
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

impl XaiRealtimeNormalizer {
    pub fn normalize(&mut self, event: Json) -> Result<Json, XaiRealtimeError> {
        let event_type = event_type(&event).map(str::to_owned);
        let kind = event_type.as_deref();
        let normalized = normalize_event(event, kind, &mut self.parts)?;
        let repaired = repair_tool_role(normalized, kind);
        let indexed = inject_missing_indices(repaired, kind);
        Ok(normalize_response_usage(indexed, kind))
    }
}
