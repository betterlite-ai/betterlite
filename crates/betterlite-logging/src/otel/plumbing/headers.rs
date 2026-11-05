use std::collections::BTreeMap;

use percent_encoding::percent_decode_str;

const SPACE_OR_TAB: [char; 2] = [' ', '\t'];

const fn is_key_byte(byte: u8) -> bool {
    matches!(
        byte,
        0x21 | 0x23..=0x27 | 0x2a | 0x2b | 0x2d | 0x2e | 0x30..=0x39 | 0x41..=0x5a | 0x5e..=0x7a | 0x7c | 0x7e
    )
}

const fn is_value_byte(byte: u8, liberal: bool) -> bool {
    matches!(byte, 0x21 | 0x23..=0x2b | 0x2d..=0x3a | 0x3c..=0x5b | 0x5d..=0x7e)
        || (liberal && byte == 0x20)
}

fn split_pair(header: &str, liberal: bool) -> Option<(&str, &str)> {
    let (raw_name, raw_value) = header.split_once('=')?;
    let name = raw_name.trim_matches(SPACE_OR_TAB);
    let value = raw_value.trim_matches(SPACE_OR_TAB);
    let name_valid = !name.is_empty() && name.bytes().all(is_key_byte);
    let value_valid = value.bytes().all(|byte| is_value_byte(byte, liberal));
    (name_valid && value_valid).then_some((name, value))
}

// OTLP header strings are W3C Baggage encoded; a value that is not stays usable as written.
#[must_use]
pub fn parse_headers(raw: Option<&str>) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    for line in raw.unwrap_or_default().split(',') {
        let header = line.trim();
        let strict = split_pair(header, false).map(|(name, value)| {
            (
                percent_decode_str(name).decode_utf8_lossy().into_owned(),
                percent_decode_str(value).decode_utf8_lossy().into_owned(),
            )
        });
        let pair = strict.or_else(|| {
            split_pair(header, true).map(|(name, value)| (name.to_owned(), value.to_owned()))
        });
        if let Some((name, value)) = pair {
            headers.insert(name.trim().to_lowercase(), value.trim().to_owned());
        }
    }
    headers
}

#[path = "headers/tests.rs"]
mod tests;
