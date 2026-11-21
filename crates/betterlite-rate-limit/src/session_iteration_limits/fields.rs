use betterlite_models::decisions::Json;

// the two request buckets _get_session_id reads, in the order it reads them
const SESSION_ID_BUCKETS: [&str; 2] = ["metadata", "litellm_metadata"];
const SESSION_ID: &str = "session_id";

// _get_session_id: metadata first, then litellm_metadata, and none when neither bucket holds a text id
pub(crate) fn session_id_from(data: &Json) -> Option<&str> {
    let Json::Object(fields) = data else {
        return None;
    };
    SESSION_ID_BUCKETS.iter().find_map(|bucket| {
        let Some(Json::Object(entries)) = fields.get(*bucket) else {
            return None;
        };
        let Some(Json::Text(session_id)) = entries.get(SESSION_ID) else {
            return None;
        };
        Some(session_id.as_str())
    })
}
