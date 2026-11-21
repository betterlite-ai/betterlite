use betterlite_models::decisions::Json;

// the two request buckets _get_session_id reads, in the order it reads them
const SESSION_ID_BUCKETS: [&str; 2] = ["metadata", "litellm_metadata"];
const SESSION_ID: &str = "session_id";

// _get_session_id str()s the id it finds, so a numeric id names the same bucket its text form would;
// a null, boolean, list or object id names no session, as identity_descriptors::session_resolution reads it
fn session_text(value: &Json) -> Option<String> {
    match value {
        Json::Text(text) => Some(text.clone()),
        Json::Number(number) => Some(number.to_string()),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// _get_session_id: metadata first, then litellm_metadata, and none when neither bucket holds an id
pub(crate) fn session_id_from(data: &Json) -> Option<String> {
    let Json::Object(fields) = data else {
        return None;
    };
    SESSION_ID_BUCKETS.iter().find_map(|bucket| {
        let Some(Json::Object(entries)) = fields.get(*bucket) else {
            return None;
        };
        entries.get(SESSION_ID).and_then(session_text)
    })
}
