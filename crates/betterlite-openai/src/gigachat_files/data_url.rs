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

// Python's b64decode ignores characters outside the alphabet, '=' among the data, and extra '=' at the end
pub(super) fn decode_payload(payload: &str) -> Option<Vec<u8>> {
    if !payload.is_ascii() {
        return None;
    }
    let kept: String = payload
        .bytes()
        .filter(|byte| BASE64_ALPHABET.contains(byte) || *byte == b'=')
        .map(char::from)
        .collect();
    let without_padding = kept.trim_end_matches('=');
    let data = without_padding.replace('=', "");
    let padding = kept.len().checked_sub(without_padding.len())?;
    let remainder = data.len().checked_rem(4)?;
    let needed = if remainder == 0 {
        0_usize
    } else {
        4_usize.checked_sub(remainder)?
    };
    // Python refuses one data character over a quad and a padding shorter than the quad needs
    if remainder == 1 || padding < needed {
        return None;
    }
    let mut canonical = data;
    canonical.push_str(&"=".repeat(needed));
    DATA_URL_BASE64.decode(canonical).ok()
}
