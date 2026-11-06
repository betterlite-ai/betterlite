pub mod azure;
pub mod bedrock;
pub mod managed;
pub mod nvidia;

mod permissive_base64;

mod tests;

pub const MAX_ENCODED_BYTES: usize = 64 * 1024;
pub const MAX_DECODED_BYTES: usize = 48 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Video,
    Character,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelHint {
    pub custom_llm_provider: Option<String>,
    pub model_id: Option<String>,
}

#[must_use]
pub fn decode_resource_hint(kind: ResourceKind, encoded: &[u8]) -> Option<ModelHint> {
    if encoded.len() > MAX_ENCODED_BYTES {
        tracing::debug!(
            bytes = encoded.len(),
            "resource hint exceeds the encoded size limit"
        );
        return None;
    }
    let Ok(text) = str::from_utf8(encoded) else {
        tracing::debug!("resource hint is not valid UTF-8");
        return None;
    };
    let (prefix, id_label) = match kind {
        ResourceKind::Video => (managed::VIDEO_ID_PREFIX, "video_id:"),
        ResourceKind::Character => (managed::CHARACTER_ID_PREFIX, "character_id:"),
    };
    match managed::decode_parts(text, prefix, id_label, Some(MAX_DECODED_BYTES)) {
        managed::PartsOutcome::Decoded(parts) => Some(ModelHint {
            custom_llm_provider: parts.custom_llm_provider,
            model_id: parts.model_id,
        }),
        managed::PartsOutcome::Native => None,
        managed::PartsOutcome::Undecodable => {
            tracing::debug!("resource hint could not be decoded");
            None
        }
    }
}
