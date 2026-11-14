use betterlite_prompts::arguments::json::{Json, Object};

use super::error::BodyError;

pub fn merge_extra_body(body: &mut Object, params: &mut Object) -> Result<(), BodyError> {
    let extra_body = match params.shift_remove("extra_body") {
        None | Some(Json::Null) => return Ok(()),
        Some(Json::Object(extra_body)) => extra_body,
        Some(_) => return Err(BodyError::Field("extra_body", "an object")),
    };
    for (key, value) in extra_body {
        // LiteLLM strips its own bookkeeping keys before the caller's body reaches the provider
        if key == "cache" || key == "tags" {
            continue;
        }
        let merged = match (body.get_mut(&key), value) {
            (Some(Json::Object(existing)), Json::Object(given)) => {
                existing.extend(given);
                continue;
            }
            (_, given) => given,
        };
        body.insert(key, merged);
    }
    Ok(())
}

// The Maps tool rejects a response mime type, so the legacy schema fields move into responseFormat text
pub fn rewrite_maps_response_format(body: &mut Object) {
    let Some(Json::List(tools)) = body.get("tools") else {
        return;
    };
    if !tools
        .iter()
        .any(|tool| matches!(tool, Json::Object(fields) if fields.contains_key("googleMaps")))
    {
        return;
    }
    let Some(Json::Object(config)) = body.get_mut("generationConfig") else {
        return;
    };
    if config.get("response_mime_type").and_then(Json::as_str) != Some("application/json") {
        return;
    }
    let schema = config
        .shift_remove("response_json_schema")
        .filter(|value| !matches!(value, Json::Null))
        .or_else(|| config.shift_remove("response_schema"))
        .filter(|value| !matches!(value, Json::Null));
    config.shift_remove("response_mime_type");
    let mut text = Object::new();
    text.insert(
        "mimeType".to_owned(),
        Json::Text("APPLICATION_JSON".to_owned()),
    );
    if let Some(schema_value) = schema {
        text.insert("schema".to_owned(), schema_value);
    }
    let response_format = Object::from_iter([("text".to_owned(), Json::Object(text))]);
    config.insert("responseFormat".to_owned(), Json::Object(response_format));
}
