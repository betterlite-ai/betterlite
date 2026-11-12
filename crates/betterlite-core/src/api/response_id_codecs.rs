use crate::api::api_boundaries::ApiLimits;

pub mod container;
pub mod encrypted;
mod lenient;
pub mod response;
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseIdCodecKind {
    Response,
    Container,
    EncryptedItem,
    EncryptedContent,
}

impl ResponseIdCodecKind {
    // the fuzzer's first byte picks the codec, so every byte value reaches one of the four
    #[must_use]
    pub const fn from_selector(selector: u8) -> Self {
        match selector.checked_rem(4) {
            Some(0) => Self::Response,
            Some(1) => Self::Container,
            Some(2) => Self::EncryptedItem,
            _ => Self::EncryptedContent,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecDecoding {
    pub custom_llm_provider: Option<String>,
    pub model_id: Option<String>,
    pub restored_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CodecError {
    #[error("the response id is {observed} bytes; the maximum is {limit}")]
    TooLarge { observed: usize, limit: usize },
}

pub fn decode_response_id_codecs(
    kind: ResponseIdCodecKind,
    bytes: &[u8],
    limits: ApiLimits,
) -> Result<CodecDecoding, CodecError> {
    if bytes.len() > limits.max_json_bytes {
        return Err(CodecError::TooLarge {
            observed: bytes.len(),
            limit: limits.max_json_bytes,
        });
    }
    if bytes.len() > limits.max_event_bytes {
        return Err(CodecError::TooLarge {
            observed: bytes.len(),
            limit: limits.max_event_bytes,
        });
    }
    Ok(match kind {
        ResponseIdCodecKind::Response => response::decode_entry(bytes),
        ResponseIdCodecKind::Container => container::decode_entry(bytes),
        ResponseIdCodecKind::EncryptedItem => encrypted::decode_item_entry(bytes),
        ResponseIdCodecKind::EncryptedContent => encrypted::decode_content_entry(bytes),
    })
}
