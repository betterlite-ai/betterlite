use std::str::from_utf8;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use super::CodecDecoding;
use super::lenient;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedItemId {
    pub model_id: String,
    pub item_id: String,
}

#[must_use]
pub fn build_encrypted_item_id(model_id: &str, item_id: &str) -> String {
    let assembled = format!("litellm:model_id:{model_id};item_id:{item_id}");
    let encoded = STANDARD.encode(assembled);
    format!("encitem_{encoded}")
}

#[must_use]
pub fn decode_encrypted_item_id(encoded_id: &str) -> Option<EncryptedItemId> {
    let cleaned = encoded_id.strip_prefix("encitem_")?;
    let mut padded = cleaned.to_owned();
    let remainder = padded.chars().count().checked_rem(4)?;
    if remainder != 0 {
        padded.push_str(&"=".repeat(4_usize.saturating_sub(remainder)));
    }
    let decoded = lenient::decode_text(&padded).ok()?;
    let (model_part, item_part) = decoded.split_once(';')?;
    Some(EncryptedItemId {
        model_id: model_part.replace("litellm:model_id:", ""),
        item_id: item_part.replace("item_id:", ""),
    })
}

#[must_use]
pub fn wrap_encrypted_content(encrypted_content: &str, model_id: &str) -> String {
    let metadata = format!("model_id:{model_id}");
    let encoded_metadata = STANDARD.encode(metadata);
    format!("litellm_enc:{encoded_metadata};{encrypted_content}")
}

#[must_use]
pub fn unwrap_encrypted_content(wrapped_content: &str) -> (Option<String>, String) {
    if !wrapped_content.starts_with("litellm_enc:") {
        return (None, wrapped_content.to_owned());
    }
    let Some((metadata, original_content)) = wrapped_content.split_once(';') else {
        return (None, wrapped_content.to_owned());
    };
    let metadata_b64 = metadata.replace("litellm_enc:", "");
    let mut padded = metadata_b64;
    match padded.chars().count().checked_rem(4) {
        Some(0) => {}
        Some(remainder) => padded.push_str(&"=".repeat(4_usize.saturating_sub(remainder))),
        None => return (None, wrapped_content.to_owned()),
    }
    let Ok(decoded_metadata) = lenient::decode_text(&padded) else {
        return (None, wrapped_content.to_owned());
    };
    (
        Some(decoded_metadata.replace("model_id:", "")),
        original_content.to_owned(),
    )
}

pub(super) fn decode_item_entry(bytes: &[u8]) -> CodecDecoding {
    match from_utf8(bytes).ok().and_then(decode_encrypted_item_id) {
        Some(item) => CodecDecoding {
            custom_llm_provider: None,
            model_id: Some(item.model_id),
            restored_id: Some(item.item_id),
        },
        None => CodecDecoding {
            custom_llm_provider: None,
            model_id: None,
            restored_id: None,
        },
    }
}

pub(super) fn decode_content_entry(bytes: &[u8]) -> CodecDecoding {
    let Ok(wrapped_content) = from_utf8(bytes) else {
        return CodecDecoding {
            custom_llm_provider: None,
            model_id: None,
            restored_id: None,
        };
    };
    let (model_id, content) = unwrap_encrypted_content(wrapped_content);
    CodecDecoding {
        custom_llm_provider: None,
        model_id,
        restored_id: Some(content),
    }
}
