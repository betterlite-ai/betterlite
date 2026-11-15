use base64::Engine as _;
use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use serde_json::{Map, Value, json};

const VIDEO_ID_PREFIX: &str = "video_";
const CHARACTER_ID_PREFIX: &str = "character_";
const PROVIDER_FIELD: &str = "litellm:custom_llm_provider:";
const MODEL_FIELD: &str = "model_id:";
const VIDEO_FIELD: &str = "video_id:";
const CHARACTER_FIELD: &str = "character_id:";
const QUOTED_FIELDS: &str = "litellm:quoted:";

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
        } else if character == '=' && data.len().checked_rem(4)? >= 2 {
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

fn decode_fields(encoded: &str, prefix: &str, value_field: &str) -> Option<DecodedFields> {
    let cleaned = encoded.replace(prefix, "");
    let decoded = decode_base64_text(&cleaned)?;
    if let Some(fields) = decode_quoted_fields(&decoded) {
        return Some(fields);
    }
    Some(split_fields(&decoded, encoded, value_field))
}

fn split_fields(decoded: &str, encoded: &str, value_field: &str) -> DecodedFields {
    let parts: Vec<&str> = decoded.split(';').collect();
    let (provider, model, value) = match parts.as_slice() {
        [provider, model, value, ..] => (Some(*provider), Some(*model), Some(*value)),
        _ => (None, None, None),
    };
    DecodedFields {
        custom_llm_provider: provider.map(|part| part.replace(PROVIDER_FIELD, "")),
        model_id: model.map(|part| part.replace(MODEL_FIELD, "")),
        value: value.map_or_else(|| encoded.to_owned(), |part| part.replace(value_field, "")),
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

fn encode_payload(provider: &str, model_id: &str, value: &str, value_field: &str) -> String {
    let assembled =
        format!("{PROVIDER_FIELD}{provider};{MODEL_FIELD}{model_id};{value_field}{value}");
    let fields = split_fields(&assembled, &assembled, value_field);
    if fields.custom_llm_provider.as_deref() == Some(provider)
        && fields.model_id.as_deref() == Some(model_id)
        && fields.value == value
    {
        return assembled;
    }
    format!(
        "{QUOTED_FIELDS}{}",
        json!({"custom_llm_provider": provider, "model_id": model_id, "value": value})
    )
}

fn fallback_video_id(encoded_video_id: &str) -> DecodedVideoId {
    DecodedVideoId {
        custom_llm_provider: None,
        model_id: None,
        video_id: encoded_video_id.to_owned(),
    }
}

fn fallback_character_id(encoded_character_id: &str) -> DecodedCharacterId {
    DecodedCharacterId {
        custom_llm_provider: None,
        model_id: None,
        character_id: encoded_character_id.to_owned(),
    }
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
    if decode_video_id_with_provider(video_id)
        .custom_llm_provider
        .is_some()
    {
        return video_id.to_owned();
    }
    let payload = encode_payload(
        provider,
        model_id.unwrap_or_default(),
        video_id,
        VIDEO_FIELD,
    );
    format!("{VIDEO_ID_PREFIX}{}", ID_BASE64.encode(payload.as_bytes()))
}

#[must_use]
pub fn decode_video_id_with_provider(encoded_video_id: &str) -> DecodedVideoId {
    if encoded_video_id.is_empty() || !encoded_video_id.starts_with(VIDEO_ID_PREFIX) {
        return fallback_video_id(encoded_video_id);
    }
    match decode_fields(encoded_video_id, VIDEO_ID_PREFIX, VIDEO_FIELD) {
        Some(fields) => DecodedVideoId {
            custom_llm_provider: fields.custom_llm_provider,
            model_id: fields.model_id,
            video_id: fields.value,
        },
        None => {
            tracing::debug!(video_id = %encoded_video_id, "error decoding provider video id");
            fallback_video_id(encoded_video_id)
        }
    }
}

#[must_use]
pub fn encode_character_id(character_id: &str, provider: &str, model_id: Option<&str>) -> String {
    if provider.is_empty() || character_id.is_empty() {
        return character_id.to_owned();
    }
    if decode_character_id(character_id)
        .custom_llm_provider
        .is_some()
    {
        return character_id.to_owned();
    }
    let payload = encode_payload(
        provider,
        model_id.unwrap_or_default(),
        character_id,
        CHARACTER_FIELD,
    );
    format!(
        "{CHARACTER_ID_PREFIX}{}",
        ID_BASE64.encode(payload.as_bytes())
    )
}

#[must_use]
pub fn decode_character_id(encoded_character_id: &str) -> DecodedCharacterId {
    if encoded_character_id.is_empty() || !encoded_character_id.starts_with(CHARACTER_ID_PREFIX) {
        return fallback_character_id(encoded_character_id);
    }
    match decode_fields(encoded_character_id, CHARACTER_ID_PREFIX, CHARACTER_FIELD) {
        Some(fields) => DecodedCharacterId {
            custom_llm_provider: fields.custom_llm_provider,
            model_id: fields.model_id,
            character_id: fields.value,
        },
        None => {
            tracing::debug!(character_id = %encoded_character_id, "error decoding provider character id");
            fallback_character_id(encoded_character_id)
        }
    }
}

#[must_use]
pub fn decode_provider_video_id(bytes: &[u8]) -> DecodedProviderId {
    let Ok(text) = std::str::from_utf8(bytes) else {
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
    let Some(Value::Array(characters)) = request.get("characters").cloned() else {
        return;
    };
    let rewritten = characters
        .into_iter()
        .map(|character| match character {
            Value::Object(mut fields) => {
                let id = fields.get("id").and_then(Value::as_str).map(str::to_owned);
                if let Some(id) = id {
                    let original = decode_character_id(&id).character_id;
                    fields.insert("id".to_owned(), Value::String(original));
                }
                Value::Object(fields)
            }
            other => other,
        })
        .collect();
    request.insert("characters".to_owned(), Value::Array(rewritten));
}
