use axum::http::{HeaderMap, header};
use secrecy::SecretString;

use betterlite_auth::auth_request_models::MODEL_ROUTING_HEADER_NAME;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::auth::parse_key_header;

pub(crate) fn extract_key(headers: &HeaderMap) -> Option<SecretString> {
    for name in [
        "x-litellm-api-key",
        header::AUTHORIZATION.as_str(),
        "api-key",
        "x-api-key",
        "x-goog-api-key",
        "ocp-apim-subscription-key",
    ] {
        let Some(value) = headers.get(name) else {
            continue;
        };
        if let Some(key) = parse_key_header(name, value.as_bytes()) {
            return Some(key);
        }
    }
    None
}

pub(crate) fn headers_json(headers: &HeaderMap) -> JsonObject {
    let mut object = JsonObject::new();
    if let Some(value) = headers.get(MODEL_ROUTING_HEADER_NAME) {
        // LiteLLM decodes header bytes as latin-1, and only this header feeds the model candidates
        let text: String = value
            .as_bytes()
            .iter()
            .map(|byte| char::from(*byte))
            .collect();
        object.insert(MODEL_ROUTING_HEADER_NAME.to_owned(), Json::Text(text));
    }
    object
}
