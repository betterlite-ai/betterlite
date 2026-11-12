use base64::{
    Engine as _, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use error::ManagedResourceIdsError;
use std::str::from_utf8;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimits, JsonBoundary};

pub mod error;
mod tests;

pub const UNIFIED_ID_PREFIX: &str = "litellm_proxy:";

// Python's b64decode discards every character outside the alphabet and tolerates nonzero trailing bits
const LENIENT_BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireNone),
);

// str.strip() in Python also removes U+001C to U+001F, which char::is_whitespace keeps
const STRIP_SPACE: [char; 29] = [
    '\u{9}', '\u{a}', '\u{b}', '\u{c}', '\u{d}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{1f}', ' ',
    '\u{85}', '\u{a0}', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}',
    '\u{2005}', '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}', '\u{200a}', '\u{2028}', '\u{2029}',
    '\u{202f}', '\u{205f}', '\u{3000}',
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassthroughManagedIdScope {
    Openai,
    Azure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedIdParts {
    pub resource_type: Option<String>,
    pub unified_uuid: Option<String>,
    pub target_model_names: Vec<String>,
    pub provider_resource_id: Option<String>,
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedResourceIds {
    pub decoded: Option<String>,
    pub parts: Option<UnifiedIdParts>,
}

#[must_use]
pub fn passthrough_provider_scope(custom_llm_provider: &str) -> Option<PassthroughManagedIdScope> {
    let provider = custom_llm_provider.to_lowercase();
    // a bare name or the last dotted segment decides, as with endswith(".azure")
    let name = provider
        .rsplit_once('.')
        .map_or(provider.as_str(), |(_, last)| last);
    match name {
        "azure" | "azure_ai" => Some(PassthroughManagedIdScope::Azure),
        "openai" => Some(PassthroughManagedIdScope::Openai),
        _ => None,
    }
}

fn decode_lenient_base64(text: &str) -> Option<Vec<u8>> {
    if !text.is_ascii() {
        return None;
    }
    let mut symbols = Vec::with_capacity(text.len());
    let mut pads = 0_usize;
    for byte in text.bytes() {
        let symbol = match byte {
            b'=' => {
                pads = pads.saturating_add(1);
                continue;
            }
            b'-' => b'+',
            b'_' => b'/',
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' => byte,
            _ => continue,
        };
        symbols.push(symbol);
        pads = 0;
    }
    // Python appends -len % 4 pad characters before decoding, and they count toward the quad
    let all_pads = pads.saturating_add(text.len().wrapping_neg().rem_euclid(4));
    match symbols.len().rem_euclid(4) {
        remainder @ (2 | 3) if remainder.saturating_add(all_pads) < 4 => None,
        _ => LENIENT_BASE64.decode(symbols).ok(),
    }
}

#[must_use]
pub fn is_base64_encoded_unified_id(resource_id: &str, prefix: &str) -> Option<String> {
    let bytes = decode_lenient_base64(resource_id)?;
    let decoded = String::from_utf8(bytes).ok()?;
    decoded.starts_with(prefix).then_some(decoded)
}

#[must_use]
pub fn decode_unified_id(encoded_unified_id: &str) -> Option<String> {
    let bytes = decode_lenient_base64(encoded_unified_id)?;
    let decoded = String::from_utf8(bytes).ok()?;
    decoded.starts_with(UNIFIED_ID_PREFIX).then_some(decoded)
}

// the first place `key` is followed by at least one character before the next `;`
fn first_field<'text>(text: &'text str, key: &str, field_boundary: bool) -> Option<&'text str> {
    text.match_indices(key).find_map(|(start, _)| {
        let before = text.get(..start)?;
        if field_boundary && !before.is_empty() && !before.ends_with(';') {
            return None;
        }
        let value = text.get(start..)?.strip_prefix(key)?.split(';').next()?;
        (!value.is_empty()).then_some(value)
    })
}

#[must_use]
pub fn target_model_names(unified_id: &str) -> Vec<String> {
    let decoded = is_base64_encoded_unified_id(unified_id, UNIFIED_ID_PREFIX);
    let text = decoded.as_deref().unwrap_or(unified_id);
    first_field(text, "target_model_names,", false)
        .map(|names| {
            names
                .split(',')
                .map(|name| name.trim_matches(STRIP_SPACE).to_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[must_use]
pub fn resource_type(unified_id: &str) -> Option<String> {
    let decoded = is_base64_encoded_unified_id(unified_id, UNIFIED_ID_PREFIX);
    let text = decoded.as_deref().unwrap_or(unified_id);
    let value = first_field(text, UNIFIED_ID_PREFIX, false)?;
    Some(value.trim_matches(STRIP_SPACE).to_owned())
}

#[must_use]
pub fn unified_uuid(unified_id: &str) -> Option<String> {
    let decoded = is_base64_encoded_unified_id(unified_id, UNIFIED_ID_PREFIX);
    let text = decoded.as_deref().unwrap_or(unified_id);
    let value = first_field(text, "unified_id,", false)?;
    Some(value.trim_matches(STRIP_SPACE).to_owned())
}

#[must_use]
pub fn model_id(unified_id: &str) -> Option<String> {
    let decoded = is_base64_encoded_unified_id(unified_id, UNIFIED_ID_PREFIX);
    let text = decoded.as_deref().unwrap_or(unified_id);
    let value = first_field(text, "model_id,", true)?;
    Some(value.trim_matches(STRIP_SPACE).to_owned())
}

#[must_use]
pub fn provider_resource_id(unified_id: &str) -> Option<String> {
    let decoded = is_base64_encoded_unified_id(unified_id, UNIFIED_ID_PREFIX);
    let text = decoded.as_deref().unwrap_or(unified_id);
    let value = ["resource_id,", "vector_store_id,", "file_id,"]
        .iter()
        .find_map(|key| first_field(text, key, false))?;
    Some(value.trim_matches(STRIP_SPACE).to_owned())
}

#[must_use]
pub fn parse_unified_id(unified_id: &str) -> Option<UnifiedIdParts> {
    let decoded = decode_unified_id(unified_id);
    let text = match decoded.as_deref() {
        Some(text) => text,
        None if unified_id.starts_with(UNIFIED_ID_PREFIX) => unified_id,
        None => return None,
    };
    Some(UnifiedIdParts {
        resource_type: resource_type(text),
        unified_uuid: unified_uuid(text),
        target_model_names: target_model_names(text),
        provider_resource_id: provider_resource_id(text),
        model_id: model_id(text),
    })
}

pub fn decode_managed_resource_ids(
    bytes: &[u8],
    limits: ApiLimits,
) -> Result<ManagedResourceIds, ManagedResourceIdsError> {
    // ids arrive inside request bodies, so the JSON cap bounds them
    limits.check(ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: bytes.len(),
    }))?;
    let text = from_utf8(bytes).map_err(|source| ManagedResourceIdsError::NotUtf8 { source })?;
    Ok(ManagedResourceIds {
        decoded: decode_unified_id(text),
        parts: parse_unified_id(text),
    })
}
