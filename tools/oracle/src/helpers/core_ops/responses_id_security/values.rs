use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_id_security::owned_id::{
    DecodedResponseId, OwnedIdCipher as _, decode_owned_response_id,
};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_secrets::secret_codec::StoredValue;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Number, Value, json};

use super::{Cipher, EventSpec, NestedSpec, ValueSpec, failure};

#[derive(Debug, Clone, Copy)]
pub(crate) enum BuiltEvent {
    Lifecycle,
    Generic,
}

pub(crate) struct EventView {
    pub(crate) top: Option<String>,
    pub(crate) nested: Option<String>,
    pub(crate) advertised: Option<String>,
}

pub(crate) fn materialize(cipher: &Cipher, spec: &ValueSpec) -> Result<Json, Box<HelperError>> {
    match spec {
        ValueSpec::Text { text } => Ok(Json::Text(text.clone())),
        ValueSpec::Number { number } => Ok(Json::Number(Number::from(*number))),
        ValueSpec::Null => Ok(Json::Null),
        ValueSpec::Owned {
            original,
            user,
            team,
            prefix,
            suffix,
        } => {
            let payload = format!(
                "litellm_proxy:responses_api:response_id:{original};user_id:{user};team_id:{team}"
            );
            sealed_text(cipher, &payload, prefix, suffix)
        }
        ValueSpec::Sealed {
            text,
            prefix,
            suffix,
        } => sealed_text(cipher, text, prefix, suffix),
    }
}

// the wire format LiteLLM advertises: resp_ plus the base64url SecretBox ciphertext
fn sealed_text(
    cipher: &Cipher,
    payload: &str,
    prefix: &str,
    suffix: &str,
) -> Result<Json, Box<HelperError>> {
    let stored = StoredValue::<()>::Text(SecretString::from(payload.to_owned()));
    let sealed = cipher
        .encrypt_value(stored, None)
        .map_err(|error| failure("EncryptFailed", error.to_string()))?;
    let StoredValue::Text(text) = sealed else {
        return Err(failure(
            "EncryptFailed",
            "the cipher returned a non-text value".to_owned(),
        ));
    };
    Ok(Json::Text(format!(
        "{prefix}resp_{}{suffix}",
        text.expose_secret()
    )))
}

pub(crate) fn project(value: &Json, cipher: &Cipher) -> Result<String, Box<HelperError>> {
    match value {
        Json::Text(text) => Ok(project_text(text, cipher)),
        other => serde_json::to_string(other)
            .map(|text| format!("json:{text}"))
            .map_err(|error| failure("SerializeFailed", error.to_string())),
    }
}

// a projection both sides can compare: the decrypted identity, or the plain text
fn project_text(text: &str, cipher: &Cipher) -> String {
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

pub(crate) fn project_nested(value: &Json, cipher: &Cipher) -> Result<String, Box<HelperError>> {
    match value {
        Json::Object(fields) => match fields.get("id") {
            Some(id) => project(id, cipher),
            None => project(value, cipher),
        },
        other => project(other, cipher),
    }
}

pub(crate) fn nested_json(
    cipher: &Cipher,
    nested: &NestedSpec,
) -> Result<Option<Json>, Box<HelperError>> {
    match nested {
        NestedSpec::Absent => Ok(None),
        NestedSpec::Null => Ok(Some(Json::Null)),
        NestedSpec::Number { number } => Ok(Some(Json::Number(Number::from(*number)))),
        NestedSpec::Mapping { id } => {
            let mut fields = IndexMap::new();
            fields.insert("id".to_owned(), materialize(cipher, id)?);
            Ok(Some(Json::Object(fields)))
        }
    }
}

pub(crate) fn build_event(
    cipher: &Cipher,
    spec: &EventSpec,
) -> Result<(ResponsesAPIStreamingResponse, BuiltEvent), Box<HelperError>> {
    match spec {
        EventSpec::Lifecycle {
            response_id,
            top_id,
        } => {
            let id = materialize(cipher, response_id)?;
            let mut event = json!({
                "type": "response.created",
                "response": { "id": id, "created_at": 0, "output": [] },
            });
            if let Some(top) = top_id {
                insert_field(&mut event, "id", &materialize(cipher, top)?)?;
            }
            Ok((parse_event(event)?, BuiltEvent::Lifecycle))
        }
        EventSpec::Generic { top_id, nested } => {
            let mut event = json!({ "type": "response.custom_event" });
            if let Some(value) = nested_json(cipher, nested)? {
                insert_field(&mut event, "response", &value)?;
            }
            if let Some(top) = top_id {
                insert_field(&mut event, "id", &materialize(cipher, top)?)?;
            }
            Ok((parse_event(event)?, BuiltEvent::Generic))
        }
    }
}

fn insert_field(event: &mut Value, name: &str, value: &Json) -> Result<(), Box<HelperError>> {
    let Some(fields) = event.as_object_mut() else {
        return Err(failure(
            "InvalidInput",
            "the event is not an object".to_owned(),
        ));
    };
    let converted = serde_json::to_value(value)
        .map_err(|error| failure("SerializeFailed", error.to_string()))?;
    fields.insert(name.to_owned(), converted);
    Ok(())
}

fn parse_event(event: Value) -> Result<ResponsesAPIStreamingResponse, Box<HelperError>> {
    serde_json::from_value(event).map_err(|error| failure("InvalidInput", error.to_string()))
}

pub(crate) fn view_event(
    value: &Value,
    built: BuiltEvent,
    cipher: &Cipher,
) -> Result<EventView, Box<HelperError>> {
    let Some(fields) = value.as_object() else {
        return Err(failure(
            "SerializeFailed",
            "the event did not serialize as an object".to_owned(),
        ));
    };
    let top = match fields.get("id") {
        Some(id) => Some(project(&as_json(id)?, cipher)?),
        None => None,
    };
    let nested = match built {
        BuiltEvent::Lifecycle => {
            match fields
                .get("response")
                .and_then(|response| response.get("id"))
            {
                Some(id) => Some(project(&as_json(id)?, cipher)?),
                None => None,
            }
        }
        BuiltEvent::Generic => match fields.get("response") {
            Some(response) => Some(project_nested(&as_json(response)?, cipher)?),
            None => None,
        },
    };
    Ok(EventView {
        top,
        nested,
        advertised: advertised_id(fields),
    })
}

// the id a client sees on the event: a top-level string id, else the nested response id
fn advertised_id(fields: &serde_json::Map<String, Value>) -> Option<String> {
    if let Some(top) = fields.get("id").and_then(Value::as_str) {
        return Some(top.to_owned());
    }
    fields
        .get("response")
        .and_then(|response| response.get("id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn as_json(value: &Value) -> Result<Json, Box<HelperError>> {
    serde_json::from_value(value.clone())
        .map_err(|error| failure("SerializeFailed", error.to_string()))
}
