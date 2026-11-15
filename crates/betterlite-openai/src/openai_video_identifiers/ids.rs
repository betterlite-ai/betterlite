use std::str;

use base64::Engine as _;
use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use serde_json::{Map, Value};

const VIDEO_ID_PREFIX: &str = "video_";
const CHARACTER_ID_PREFIX: &str = "character_";
const PROVIDER_FIELD: &str = "litellm:custom_llm_provider:";
const MODEL_FIELD: &str = "model_id:";
const VIDEO_FIELD: &str = "video_id:";
const CHARACTER_FIELD: &str = "character_id:";
const QUOTED_FIELDS: &str = "litellm:quoted:";

const VIDEO_KIND: IdKind = IdKind {
    prefix: VIDEO_ID_PREFIX,
    value_field: VIDEO_FIELD,
    failure_message: "error decoding provider video id",
};
const CHARACTER_KIND: IdKind = IdKind {
    prefix: CHARACTER_ID_PREFIX,
    value_field: CHARACTER_FIELD,
    failure_message: "error decoding provider character id",
};

// Python's b64decode(validate=False) keeps nonzero trailing bits and takes the padding it is given or none
const ID_BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedVideoId {
    pub custom_llm_provider: Option<String>,
    pub model_id: Option<String>,
    pub video_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedCharacterId {
    pub custom_llm_provider: Option<String>,
    pub model_id: Option<String>,
    pub character_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedProviderId {
    Video(DecodedVideoId),
    Character(DecodedCharacterId),
    Invalid,
}

#[derive(Clone, Copy)]
struct IdKind {
    prefix: &'static str,
    value_field: &'static str,
    failure_message: &'static str,
}

struct DecodedFields {
    custom_llm_provider: Option<String>,
    model_id: Option<String>,
    value: String,
}

#[derive(serde::Deserialize)]
struct QuotedFields {
    custom_llm_provider: String,
    model_id: String,
    value: String,
}

fn add_base64_padding(value: &str) -> String {
    let missing = match value.chars().count().checked_rem(4) {
        Some(1) => "===",
        Some(2) => "==",
        Some(3) => "=",
        _ => "",
    };
    format!("{value}{missing}")
}

fn decode_base64_text(encoded: &str) -> Option<String> {
    let mut data = String::new();
    let mut pads = 0_usize;
    for character in add_base64_padding(encoded).chars() {
        if character.is_ascii_alphanumeric() || character == '+' || character == '/' {
            data.push(character);
            pads = 0;
        }
        if character == '=' && data.len().checked_rem(4)? >= 2 {
            pads = pads.checked_add(1)?;
            if data.len().checked_rem(4)?.checked_add(pads)? >= 4 {
                break;
            }
        }
    }
    let leftover = data.len().checked_rem(4)?;
    if leftover != 0 && leftover.checked_add(pads)? < 4 {
        return None;
    }
    let bytes = ID_BASE64.decode(data.as_bytes()).ok()?;
    String::from_utf8(bytes).ok()
}

fn fallback_fields(encoded: &str) -> DecodedFields {
    DecodedFields {
        custom_llm_provider: None,
        model_id: None,
        value: encoded.to_owned(),
    }
}

fn decode_fields(kind: IdKind, encoded: &str) -> Option<DecodedFields> {
    let cleaned = encoded.replace(kind.prefix, "");
    let decoded = decode_base64_text(&cleaned)?;
    if let Some(fields) = decode_quoted_fields(&decoded) {
        return Some(fields);
    }
    Some(split_fields(&decoded, encoded, kind))
}

fn split_fields(decoded: &str, encoded: &str, kind: IdKind) -> DecodedFields {
    let parts: Vec<&str> = decoded.split(';').collect();
    let (provider, model, value) = match parts.as_slice() {
        [provider, model, value, ..] => (Some(*provider), Some(*model), Some(*value)),
        _ => (None, None, None),
    };
    DecodedFields {
        custom_llm_provider: provider.map(|part| part.replace(PROVIDER_FIELD, "")),
        model_id: model.map(|part| part.replace(MODEL_FIELD, "")),
        value: value.map_or_else(
            || encoded.to_owned(),
            |part| part.replace(kind.value_field, ""),
        ),
    }
}

fn decode_quoted_fields(decoded: &str) -> Option<DecodedFields> {
    let rest = decoded.strip_prefix(QUOTED_FIELDS)?;
    let quoted: QuotedFields = serde_json::from_str(rest).ok()?;
    Some(DecodedFields {
        custom_llm_provider: Some(quoted.custom_llm_provider),
        model_id: Some(quoted.model_id),
        value: quoted.value,
    })
}

fn lookup_id(kind: IdKind, encoded: &str) -> DecodedFields {
    if !encoded.starts_with(kind.prefix) {
        return fallback_fields(encoded);
    }
    decode_fields(kind, encoded).unwrap_or_else(|| {
        tracing::debug!(id = %encoded, "{}", kind.failure_message);
        fallback_fields(encoded)
    })
}

fn encode_payload(provider: &str, model_id: &str, value: &str, kind: IdKind) -> String {
    let assembled = format!(
        "{PROVIDER_FIELD}{provider};{MODEL_FIELD}{model_id};{}{value}",
        kind.value_field
    );
    let fields = split_fields(&assembled, &assembled, kind);
    if fields.custom_llm_provider.as_deref() == Some(provider)
        && fields.model_id.as_deref() == Some(model_id)
        && fields.value == value
    {
        return assembled;
    }
    quoted_payload(provider, model_id, value)
}

fn quoted_payload(provider: &str, model_id: &str, value: &str) -> String {
    let mut quoted = Map::new();
    quoted.insert(
        "custom_llm_provider".to_owned(),
        Value::String(provider.to_owned()),
    );
    quoted.insert("model_id".to_owned(), Value::String(model_id.to_owned()));
    quoted.insert("value".to_owned(), Value::String(value.to_owned()));
    format!("{QUOTED_FIELDS}{}", Value::Object(quoted))
}

fn encode_id(kind: IdKind, value: &str, provider: &str, model_id: Option<&str>) -> String {
    if lookup_id(kind, value).custom_llm_provider.is_some() {
        return value.to_owned();
    }
    let payload = encode_payload(provider, model_id.unwrap_or_default(), value, kind);
    format!("{}{}", kind.prefix, ID_BASE64.encode(payload.as_bytes()))
}

#[must_use]
pub fn encode_video_id_with_provider(
    video_id: &str,
    provider: &str,
    model_id: Option<&str>,
) -> String {
    if provider.is_empty() || video_id.is_empty() {
        return video_id.to_owned();
    }
    encode_id(VIDEO_KIND, video_id, provider, model_id)
}

#[must_use]
pub fn decode_video_id_with_provider(encoded_video_id: &str) -> DecodedVideoId {
    let fields = lookup_id(VIDEO_KIND, encoded_video_id);
    DecodedVideoId {
        custom_llm_provider: fields.custom_llm_provider,
        model_id: fields.model_id,
        video_id: fields.value,
    }
}

#[must_use]
pub fn encode_character_id(character_id: &str, provider: &str, model_id: Option<&str>) -> String {
    if provider.is_empty() || character_id.is_empty() {
        return character_id.to_owned();
    }
    encode_id(CHARACTER_KIND, character_id, provider, model_id)
}

#[must_use]
pub fn decode_character_id(encoded_character_id: &str) -> DecodedCharacterId {
    let fields = lookup_id(CHARACTER_KIND, encoded_character_id);
    DecodedCharacterId {
        custom_llm_provider: fields.custom_llm_provider,
        model_id: fields.model_id,
        character_id: fields.value,
    }
}

#[must_use]
pub fn decode_provider_video_id(bytes: &[u8]) -> DecodedProviderId {
    let Ok(text) = str::from_utf8(bytes) else {
        return DecodedProviderId::Invalid;
    };
    if text.starts_with(CHARACTER_ID_PREFIX) {
        return DecodedProviderId::Character(decode_character_id(text));
    }
    if text.starts_with(VIDEO_ID_PREFIX) {
        return DecodedProviderId::Video(decode_video_id_with_provider(text));
    }
    DecodedProviderId::Invalid
}

pub fn rewrite_character_ids(request: &mut Map<String, Value>) {
    let Some(Value::Array(characters)) = request.get_mut("characters") else {
        return;
    };
    for character in characters {
        let Value::Object(fields) = character else {
            continue;
        };
        if let Some(encoded) = fields.get("id").and_then(Value::as_str).map(str::to_owned) {
            let original = decode_character_id(&encoded).character_id;
            fields.insert("id".to_owned(), Value::String(original));
        }
    }
}
