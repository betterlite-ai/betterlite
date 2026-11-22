use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_id_security::owned_id::{
    DecodedResponseId, OwnedIdCipher as _, decode_owned_response_id,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use serde_json::Value;

use super::{Cipher, failure};

pub(crate) struct EventView {
    pub(crate) top: Option<String>,
    pub(crate) nested: Option<String>,
    pub(crate) advertised: Option<String>,
}

pub(crate) fn project(value: &Json, cipher: &Cipher) -> Result<String, Box<HelperError>> {
    let Json::Text(text) = value else {
        return serde_json::to_string(value)
            .map(|text| format!("json:{text}"))
            .map_err(|error| failure("SerializeFailed", error.to_string()));
    };
    Ok(project_text(text, cipher))
}

// a projection both sides can compare: the decrypted identity, or the plain text
pub(crate) fn project_text(text: &str, cipher: &Cipher) -> String {
    match decode_owned_response_id(text.as_bytes(), cipher, ApiLimits::default()) {
        Ok(DecodedResponseId::Managed {
            original_id,
            user_id,
            team_id,
        }) => format!("managed:{original_id}|{user_id}|{team_id}"),
        Ok(DecodedResponseId::Fallback) => "fallback".to_owned(),
        Ok(DecodedResponseId::Unmanaged) => match decrypted_other(text, cipher) {
            Some(plaintext) => format!("decrypted:{plaintext}"),
            None => format!("plain:{text}"),
        },
        Err(_) => "malformed".to_owned(),
    }
}

// LiteLLM's decrypt_value_helper hands the segment back when the cipher refuses it
fn decrypted_other(text: &str, cipher: &Cipher) -> Option<String> {
    let segment = text.split("resp_").nth(1)?;
    let plaintext = cipher.decrypt_id(segment)?;
    if plaintext == segment {
        return None;
    }
    Some(plaintext)
}

// a nested response object projects its own id, and anything else projects itself
pub(crate) fn nested_id(value: &Json) -> &Json {
    let Json::Object(fields) = value else {
        return value;
    };
    fields.get("id").unwrap_or(value)
}

pub(crate) fn view_event(event: &Value, cipher: &Cipher) -> Result<EventView, Box<HelperError>> {
    let mut view = EventView {
        top: None,
        nested: None,
        advertised: advertised_id(event),
    };
    if let Some(id) = event.get("id") {
        view.top = Some(project(&as_json(id)?, cipher)?);
    }
    if let Some(response) = event.get("response") {
        view.nested = Some(project(nested_id(&as_json(response)?), cipher)?);
    }
    Ok(view)
}

// the id a client sees on the event: a top-level string id, else the nested response id
fn advertised_id(event: &Value) -> Option<String> {
    if let Some(top) = event.get("id").and_then(Value::as_str) {
        return Some(top.to_owned());
    }
    event
        .get("response")
        .and_then(|response| response.get("id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn as_json(value: &Value) -> Result<Json, Box<HelperError>> {
    serde_json::from_value(value.clone())
        .map_err(|error| failure("SerializeFailed", error.to_string()))
}
