use betterlite_prompts::arguments::json::{Json, Object};

use super::error::BodyError;

// LiteLLM strips its own bookkeeping keys before the caller's body reaches the provider
const INTERNAL_EXTRA_BODY_KEYS: [&str; 2] = ["cache", "tags"];
const GOOGLE_MAPS: &str = "googleMaps";
const RESPONSE_MIME_TYPE: &str = "response_mime_type";
const JSON_MIME_TYPE: &str = "application/json";
const RESPONSE_FORMAT: &str = "responseFormat";
const TEXT_RESPONSE_FORMAT: &str = "text";
const MIME_TYPE: &str = "mimeType";
const APPLICATION_JSON: &str = "APPLICATION_JSON";
const SCHEMA: &str = "schema";

pub fn merge_extra_body(body: &mut Object, optional_params: &mut Object) -> Result<(), BodyError> {
    let extra_body = match optional_params.shift_remove("extra_body") {
        None | Some(Json::Null) => return Ok(()),
        Some(Json::Object(extra_body)) => extra_body,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(BodyError::Field {
                field: "extra_body",
                expected: "an object",
            });
        }
    };
    for (key, value) in extra_body {
        if INTERNAL_EXTRA_BODY_KEYS.contains(&key.as_str()) {
            continue;
        }
        let merged = match (body.get_mut(&key), value) {
            (Some(Json::Object(existing)), Json::Object(given)) => {
                existing.extend(given);
                continue;
            }
            (_existing, given) => given,
        };
        body.insert(key, merged);
    }
    Ok(())
}

fn has_google_maps_tool(body: &Object) -> bool {
    let Some(Json::List(tools)) = body.get("tools") else {
        return false;
    };
    tools
        .iter()
        .any(|tool| matches!(tool, Json::Object(fields) if fields.contains_key(GOOGLE_MAPS)))
}

// The Maps tool rejects a response mime type, so the legacy schema fields move into the newer responseFormat text
pub fn rewrite_google_maps_response_format(body: &mut Object) {
    let mime_type = body
        .get("generationConfig")
        .and_then(Json::as_object)
        .and_then(|config| config.get(RESPONSE_MIME_TYPE))
        .and_then(Json::as_str);
    if mime_type != Some(JSON_MIME_TYPE) || !has_google_maps_tool(body) {
        return;
    }
    let Some(Json::Object(config)) = body.get_mut("generationConfig") else {
        return;
    };
    let schema = config
        .shift_remove("response_json_schema")
        .filter(|value| !matches!(value, Json::Null))
        .or_else(|| {
            config
                .shift_remove("response_schema")
                .filter(|value| !matches!(value, Json::Null))
        });
    config.shift_remove(RESPONSE_MIME_TYPE);
    let mut text = Object::new();
    text.insert(
        MIME_TYPE.to_owned(),
        Json::Text(APPLICATION_JSON.to_owned()),
    );
    if let Some(schema) = schema {
        text.insert(SCHEMA.to_owned(), schema);
    }
    let mut response_format = Object::new();
    response_format.insert(TEXT_RESPONSE_FORMAT.to_owned(), Json::Object(text));
    config.insert(RESPONSE_FORMAT.to_owned(), Json::Object(response_format));
}
