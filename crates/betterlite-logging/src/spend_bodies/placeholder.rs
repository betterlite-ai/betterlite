use super::support::{Fields, as_object};
use crate::spend_sanitize::UNKNOWN_MODEL;
use crate::spend_sanitize::raw_model::scrub_raw_model_text;
use betterlite_models::decisions::Json;
#[must_use]
pub fn placeholder_request_body(litellm_params: &Json, model_group: &str, raw_model: &str) -> Json {
    let Some(params) = as_object(litellm_params) else {
        return litellm_params.clone();
    };
    let Some(proxy) = params.get("proxy_server_request").and_then(as_object) else {
        return litellm_params.clone();
    };
    let Some(body) = proxy.get("body").and_then(as_object) else {
        return litellm_params.clone();
    };
    let mut placeholdered = body.clone();
    if placeholdered.contains_key("model") {
        placeholdered.insert("model".to_owned(), Json::Text(UNKNOWN_MODEL.to_owned()));
    }
    let new_body =
        placeholder_request_metadata(&Json::Object(placeholdered), model_group, raw_model);
    let mut proxy_fields = proxy.clone();
    proxy_fields.insert("body".to_owned(), new_body);
    let mut stored_params = params.clone();
    stored_params.insert(
        "proxy_server_request".to_owned(),
        Json::Object(proxy_fields),
    );
    Json::Object(stored_params)
}
#[must_use]
pub fn placeholder_request_metadata(
    request_body: &Json,
    model_group: &str,
    raw_model: &str,
) -> Json {
    let Some(body) = as_object(request_body) else {
        return request_body.clone();
    };
    let Some(metadata) = body.get("metadata").and_then(as_object) else {
        return request_body.clone();
    };
    let mut stored_body = body.clone();
    stored_body.insert(
        "metadata".to_owned(),
        replaced_metadata(metadata, model_group, raw_model),
    );
    Json::Object(stored_body)
}
fn replaced_metadata(metadata: &Fields, model_group: &str, raw_model: &str) -> Json {
    let mut entries = metadata.clone();
    if entries.contains_key("model_group") {
        entries.insert("model_group".to_owned(), Json::Text(model_group.to_owned()));
    }
    if let Some(Json::Object(error_information)) = metadata.get("error_information") {
        entries.insert(
            "error_information".to_owned(),
            Json::Object(scrub_error_information(error_information, raw_model)),
        );
    }
    Json::Object(entries)
}
fn scrub_error_information(information: &Fields, raw_model: &str) -> Fields {
    information
        .iter()
        .map(|(key, value)| {
            let scrubbed = match (key.as_str(), value) {
                ("error_message" | "traceback", Json::Text(text)) => {
                    Json::Text(scrub_raw_model_text(text, raw_model))
                }
                _ => value.clone(),
            };
            (key.clone(), scrubbed)
        })
        .collect()
}
