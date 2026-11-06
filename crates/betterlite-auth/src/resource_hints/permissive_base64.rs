use base64::Engine as _;
use base64::alphabet::STANDARD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

// Python's b64decode drops every byte outside the alphabet and only needs padding for a trailing partial group
const DECODER: GeneralPurpose = GeneralPurpose::new(
    &STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireCanonical),
);

pub(crate) fn decode_permissive(raw: &[u8]) -> Option<Vec<u8>> {
    let is_symbol = |byte: &u8| {
        matches!(
            *byte,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/'
        )
    };
    let mut trailing_padding = 0_usize;
    if let Some(index) = raw.iter().rposition(is_symbol) {
        for byte in raw.get(index..).unwrap_or_default() {
            if *byte == b'=' {
                trailing_padding = trailing_padding.saturating_add(1);
            }
        }
    }
    let mut symbols: Vec<u8> = raw.iter().copied().filter(is_symbol).collect();
    let enough_padding = match symbols.len() & 3 {
        0 => true,
        2 => trailing_padding >= 2,
        3 => trailing_padding >= 1,
        _ => false,
    };
    if !enough_padding {
        return None;
    }
    while !symbols.len().is_multiple_of(4) {
        symbols.push(b'=');
    }
    DECODER.decode(&symbols).ok()
}
