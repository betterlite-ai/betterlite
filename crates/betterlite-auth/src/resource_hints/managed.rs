use std::iter::repeat_n;

use super::permissive_base64::decode_permissive;

pub const VIDEO_ID_PREFIX: &str = "video_";
pub const CHARACTER_ID_PREFIX: &str = "character_";

const PROVIDER_LABEL: &str = "litellm:custom_llm_provider:";
const MODEL_LABEL: &str = "model_id:";
const VIDEO_ID_LABEL: &str = "video_id:";
const CHARACTER_ID_LABEL: &str = "character_id:";

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

pub(crate) struct DecodedParts {
    pub(crate) custom_llm_provider: String,
    pub(crate) model_id: String,
    pub(crate) resource_id: String,
}

pub(crate) enum PartsOutcome {
    Decoded(DecodedParts),
    Native,
    Undecodable,
}

#[must_use]
pub fn decode_video_id(encoded_video_id: &str) -> DecodedVideoId {
    match decode_managed_id(encoded_video_id, ManagedKind::Video) {
        Some(decoded) => DecodedVideoId {
            custom_llm_provider: Some(decoded.custom_llm_provider),
            model_id: Some(decoded.model_id),
            video_id: decoded.resource_id,
        },
        None => DecodedVideoId {
            custom_llm_provider: None,
            model_id: None,
            video_id: encoded_video_id.to_owned(),
        },
    }
}

#[must_use]
pub fn decode_character_id(encoded_character_id: &str) -> DecodedCharacterId {
    let Some(decoded) = decode_managed_id(encoded_character_id, ManagedKind::Character) else {
        return DecodedCharacterId {
            custom_llm_provider: None,
            model_id: None,
            character_id: encoded_character_id.to_owned(),
        };
    };
    DecodedCharacterId {
        custom_llm_provider: Some(decoded.custom_llm_provider),
        model_id: Some(decoded.model_id),
        character_id: decoded.resource_id,
    }
}

#[derive(Clone, Copy)]
enum ManagedKind {
    Video,
    Character,
}

fn decode_managed_id(encoded: &str, kind: ManagedKind) -> Option<DecodedParts> {
    let outcome = match kind {
        ManagedKind::Video => decode_parts(encoded, VIDEO_ID_PREFIX, VIDEO_ID_LABEL, None),
        ManagedKind::Character => {
            decode_parts(encoded, CHARACTER_ID_PREFIX, CHARACTER_ID_LABEL, None)
        }
    };
    if matches!(outcome, PartsOutcome::Undecodable) {
        tracing::debug!("managed resource id could not be decoded");
    }
    match outcome {
        PartsOutcome::Decoded(parts) => Some(parts),
        PartsOutcome::Native | PartsOutcome::Undecodable => None,
    }
}

pub(crate) fn decode_parts(
    encoded: &str,
    prefix: &str,
    id_label: &str,
    decoded_limit: Option<usize>,
) -> PartsOutcome {
    if encoded.is_empty() || !encoded.starts_with(prefix) {
        return PartsOutcome::Native;
    }
    let cleaned = encoded.replace(prefix, "");
    let mut padded = String::with_capacity(cleaned.len().saturating_add(3));
    padded.push_str(&cleaned);
    let missing = 4_usize.saturating_sub(cleaned.chars().count() & 3) & 3;
    padded.extend(repeat_n('=', missing));
    let Some(decoded) = decode_permissive(padded.as_bytes()) else {
        return PartsOutcome::Undecodable;
    };
    if decoded_limit.is_some_and(|limit| decoded.len() > limit) {
        return PartsOutcome::Undecodable;
    }
    let Ok(text) = String::from_utf8(decoded) else {
        return PartsOutcome::Undecodable;
    };
    if !text.contains(';') {
        return PartsOutcome::Native;
    }
    let parts: Vec<&str> = text.split(';').collect();
    let (Some(provider_part), Some(model_part), Some(id_part)) =
        (parts.first(), parts.get(1), parts.get(2))
    else {
        return PartsOutcome::Native;
    };
    PartsOutcome::Decoded(DecodedParts {
        custom_llm_provider: provider_part.replace(PROVIDER_LABEL, ""),
        model_id: model_part.replace(MODEL_LABEL, ""),
        resource_id: id_part.replace(id_label, ""),
    })
}
