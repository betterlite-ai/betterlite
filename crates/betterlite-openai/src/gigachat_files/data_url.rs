use base64::Engine as _;
use base64::alphabet;
use base64::engine::{GeneralPurpose, GeneralPurposeConfig};

const BASE64_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const DATA_URL_PREFIX: &str = "data:";
const DEFAULT_EXTENSION: &str = "jpg";

// Python's b64decode keeps nonzero trailing bits and requires canonical padding, which we add
const DATA_URL_BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
);

pub(super) struct DataUrl<'text> {
    pub(super) content_type: &'text str,
    pub(super) payload: &'text str,
}

pub(super) fn extension_of(content_type: &str) -> String {
    let subtype = content_type.rsplit('/').next().unwrap_or_default();
    let extension = subtype.split(';').next().unwrap_or_default();
    if extension.is_empty() {
        String::from(DEFAULT_EXTENSION)
    } else {
        extension.to_owned()
    }
}

pub(super) fn parse_data_url(text: &str) -> Option<DataUrl<'_>> {
    let rest = text.strip_prefix(DATA_URL_PREFIX)?;
    let (content_type, tail) = rest.split_once(';')?;
    if content_type.is_empty() {
        return None;
    }
    let payload = tail.strip_prefix("base64,")?.split('\n').next()?;
    if payload.is_empty() {
        return None;
    }
    Some(DataUrl {
        content_type,
        payload,
    })
}

// Python's b64decode discards characters outside the alphabet and accepts extra '=' padding
pub(super) fn decode_payload(payload: &str) -> Option<Vec<u8>> {
    if !payload.is_ascii() {
        return None;
    }
    let filtered: String = payload
        .bytes()
        .filter(|byte| BASE64_ALPHABET.contains(byte) || *byte == b'=')
        .map(char::from)
        .collect();
    let (data, padded) = match filtered.split_once('=') {
        Some((data, rest)) if rest.bytes().all(|byte| byte == b'=') => (data, true),
        Some(_) => return None,
        None => (filtered.as_str(), false),
    };
    let remainder = data.len() % 4;
    if remainder != 0 && !padded {
        return None;
    }
    let mut canonical = String::from(data);
    canonical.push_str(&"=".repeat((4 - remainder) % 4));
    DATA_URL_BASE64.decode(canonical).ok()
}
