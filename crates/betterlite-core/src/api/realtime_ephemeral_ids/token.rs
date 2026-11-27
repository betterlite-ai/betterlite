use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use super::session::text_value;

pub const TOKEN_VERSION: &str = "realtime_v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenDecodeLimits {
    pub max_bytes: usize,
}

// what _encode_realtime_token_payload writes: the ephemeral key and the expiry are passed through as they are
#[derive(Clone, Copy, Debug)]
pub struct RealtimeTokenFields<'fields> {
    pub ephemeral_key: &'fields Json,
    pub model_id: &'fields str,
    pub user_id: Option<&'fields str>,
    pub team_id: Option<&'fields str>,
    pub expires_at: &'fields Json,
    pub session_type: &'fields str,
}

// _encode_realtime_token_payload: json.dumps with separators=(",", ":"), an absent user or team id written as "", and no expiry check
#[must_use]
pub fn encode_realtime_token_payload(fields: &RealtimeTokenFields<'_>) -> String {
    let mut payload = IndexMap::new();
    payload.insert("v".to_owned(), Json::Text(TOKEN_VERSION.to_owned()));
    payload.insert("ephemeral_key".to_owned(), fields.ephemeral_key.clone());
    payload.insert(
        "model_id".to_owned(),
        Json::Text(fields.model_id.to_owned()),
    );
    payload.insert(
        "user_id".to_owned(),
        Json::Text(fields.user_id.unwrap_or_default().to_owned()),
    );
    payload.insert(
        "team_id".to_owned(),
        Json::Text(fields.team_id.unwrap_or_default().to_owned()),
    );
    payload.insert("expires_at".to_owned(), fields.expires_at.clone());
    payload.insert(
        "session_type".to_owned(),
        Json::Text(fields.session_type.to_owned()),
    );
    let mut text = String::new();
    push_json(&mut text, &Json::Object(payload));
    text
}

// json.dumps with the compact separators: python_json writes every scalar, so only the containers differ from its defaults
fn push_json(out: &mut String, value: &Json) {
    match value {
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {
            out.push_str(&python_json::dumps(value));
        }
        Json::List(items) => {
            out.push('[');
            for (position, item) in items.iter().enumerate() {
                if position != 0 {
                    out.push(',');
                }
                push_json(out, item);
            }
            out.push(']');
        }
        Json::Object(object) => {
            out.push('{');
            for (position, (key, item)) in object.iter().enumerate() {
                if position != 0 {
                    out.push(',');
                }
                push_json(out, &Json::Text(key.clone()));
                out.push(':');
                push_json(out, item);
            }
            out.push('}');
        }
    }
}

// _decode_realtime_token_payload's answer: the three checked fields named, and every other decoded field kept as sent
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DecodedRealtimeToken {
    #[serde(rename = "v")]
    pub version: String,
    pub ephemeral_key: String,
    pub model_id: String,
    #[serde(flatten)]
    pub fields: IndexMap<String, Json>,
}

// _decode_realtime_token_payload: None for a legacy raw key or anything that is not a versioned object, so the caller alone decides the fallback
#[must_use]
pub fn decode_realtime_token_payload(
    bytes: &[u8],
    limits: TokenDecodeLimits,
) -> Option<DecodedRealtimeToken> {
    if bytes.len() > limits.max_bytes {
        return None;
    }
    let Ok(Json::Object(mut fields)) = serde_json::from_slice(bytes) else {
        return None;
    };
    if text_value(fields.get("v")) != Some(TOKEN_VERSION) {
        return None;
    }
    let ephemeral_key = text_value(fields.get("ephemeral_key"))?.to_owned();
    let model_id = text_value(fields.get("model_id"))?.to_owned();
    drop(fields.shift_remove("v"));
    drop(fields.shift_remove("ephemeral_key"));
    drop(fields.shift_remove("model_id"));
    Some(DecodedRealtimeToken {
        version: TOKEN_VERSION.to_owned(),
        ephemeral_key,
        model_id,
        fields,
    })
}
