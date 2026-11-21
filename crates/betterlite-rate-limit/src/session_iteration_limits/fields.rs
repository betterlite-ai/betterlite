use betterlite_models::decisions::Json;

// the two request buckets _get_session_id reads, in the order it reads them
const SESSION_ID_BUCKETS: [&str; 2] = ["metadata", "litellm_metadata"];
const SESSION_ID: &str = "session_id";

// _get_session_id: metadata first, then litellm_metadata, and none when neither bucket holds an id.
// It str()s the id it finds, so a numeric id names the same bucket its text form would; a null,
// boolean, list or object id names no session, as identity_descriptors::session_resolution has it.
pub(crate) fn session_id_from(data: &Json) -> Option<String> {
    let Json::Object(fields) = data else {
        return None;
    };
    SESSION_ID_BUCKETS.iter().find_map(|bucket| {
        let Some(Json::Object(entries)) = fields.get(*bucket) else {
            return None;
        };
        match entries.get(SESSION_ID)? {
            Json::Text(text) => Some(text.clone()),
            Json::Number(number) => Some(number.to_string()),
            Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
        }
    })
}
