use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;

use super::text::optional_str;

// litellm.utils.get_logging_id, which composes the timestamped id Langfuse uses as the generation id
#[must_use]
pub fn logging_id(
    start_time: Option<&DateTime<Utc>>,
    response_obj: Option<&Json>,
) -> Option<String> {
    let start = start_time?;
    let Some(Json::Object(fields)) = response_obj else {
        return None;
    };
    let Json::Text(id) = fields.get("id")? else {
        return None;
    };
    let stamp = start.format("%H-%M-%S-%6f");
    Some(format!("time-{stamp}_{id}"))
}

// litellm.integrations.langfuse._lookup_ids; the response id travels in metadata so v4's hashed span ids keep v2's generation id
#[must_use]
pub fn lookup_ids(
    litellm_call_id: Option<&str>,
    response_obj: Option<&Json>,
) -> IndexMap<String, String> {
    let mut ids = IndexMap::new();
    if let Some(call_id) = litellm_call_id {
        ids.insert("litellm_call_id".to_owned(), call_id.to_owned());
    }
    let found_response_id = match response_obj {
        Some(Json::Object(fields)) => optional_str(fields.get("id")),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    };
    if let Some(response_id) = found_response_id {
        ids.insert("response_id".to_owned(), response_id);
    }
    ids
}
