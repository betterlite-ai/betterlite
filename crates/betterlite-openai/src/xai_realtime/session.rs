use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// xAI does not default turn_detection.create_response to true for server_vad as OpenAI does
#[must_use]
pub fn patch_outgoing_session(session: Json) -> Json {
    match session {
        Json::Object(mut fields) => {
            if let Some(Json::Object(turn_detection)) = fields.get_mut("turn_detection") {
                default_create_response(turn_detection);
            }
            if let Some(Json::Object(audio)) = fields.get_mut("audio") {
                patch_nested_turn_detection(audio);
            }
            Json::Object(fields)
        }
        other => other,
    }
}

fn patch_nested_turn_detection(audio: &mut IndexMap<String, Json>) {
    let Some(Json::Object(input)) = audio.get_mut("input") else {
        return;
    };
    let Some(Json::Object(turn_detection)) = input.get_mut("turn_detection") else {
        return;
    };
    default_create_response(turn_detection);
}

fn default_create_response(turn_detection: &mut IndexMap<String, Json>) {
    if matches!(turn_detection.get("type"), Some(Json::Text(kind)) if kind == "server_vad")
        && !turn_detection.contains_key("create_response")
    {
        turn_detection.insert(String::from("create_response"), Json::Bool(true));
    }
}
