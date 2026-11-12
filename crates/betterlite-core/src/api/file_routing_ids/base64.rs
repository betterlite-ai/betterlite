use base64::{
    Engine as _, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};

// CPython's urlsafe_b64decode drops bytes off the alphabet and tolerates nonzero trailing bits
const URLSAFE_LENIENT: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireNone),
);

pub(super) fn decode_urlsafe_text(text: &str) -> Option<String> {
    if !text.is_ascii() {
        return None;
    }
    let (symbols, trailing_pads) = collect_sextets(text);
    // Python pads the raw text to a multiple of four before decoding, so its own characters count
    let auto_pads = text.len().wrapping_neg().rem_euclid(4);
    let remainder = symbols.len().rem_euclid(4);
    let available_pads = trailing_pads.saturating_add(auto_pads);
    if matches!(remainder, 2 | 3) && remainder.saturating_add(available_pads) < 4 {
        return None;
    }
    let decoded = URLSAFE_LENIENT.decode(symbols).ok()?;
    String::from_utf8(decoded).ok()
}

fn collect_sextets(text: &str) -> (Vec<u8>, usize) {
    let mut symbols = Vec::with_capacity(text.len());
    let mut pads = 0_usize;
    for byte in text.bytes() {
        let sextet = match byte {
            b'=' => {
                pads = pads.saturating_add(1);
                continue;
            }
            b'-' => b'+',
            b'_' => b'/',
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' => byte,
            _ => continue,
        };
        symbols.push(sextet);
        pads = 0;
    }
    (symbols, pads)
}
