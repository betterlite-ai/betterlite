use std::collections::BTreeMap;

use percent_encoding::percent_decode_str;
use tracing::warn;

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
    for (index, line) in raw.unwrap_or_default().split(',').enumerate() {
        let header = line.trim();
        if header.is_empty() {
            continue;
        }
        let strict = split_pair(header, false).map(|(name, value)| {
            let decoded_name = percent_decode_str(name).decode_utf8_lossy();
            let decoded_value = percent_decode_str(value).decode_utf8_lossy();
            (
                decoded_name.trim().to_owned(),
                decoded_value.trim().to_owned(),
            )
        });
        let pair = strict.or_else(|| {
            split_pair(header, true).map(|(name, value)| (name.to_owned(), value.to_owned()))
        });
        let Some((name, value)) = pair else {
            // An entry that does not parse may carry a credential, so only its position is logged.
            warn!(
                position = index.saturating_add(1),
                "otel: dropping an OTLP header entry that does not parse as name=value"
            );
            continue;
        };
        headers.insert(name.trim().to_lowercase(), value.trim().to_owned());
    }
    headers
}

#[path = "headers/tests.rs"]
mod tests;
