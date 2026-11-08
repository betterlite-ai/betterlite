use base64::engine::{
    DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig, general_purpose::STANDARD,
};
use base64::{Engine as _, alphabet};

// Python's b64decode discards every character outside the alphabet and tolerates nonzero trailing bits
const PYTHON_BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireNone),
);

#[must_use]
pub fn is_base64(value: &str) -> bool {
    let Ok(decoded) = STANDARD.decode(value) else {
        return false;
    };
    STANDARD.encode(decoded) == value
}

pub(super) fn decode_lenient_ciphertext(value: &str) -> Result<Vec<u8>, base64::DecodeError> {
    let symbols = value
        .bytes()
        .filter(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'+' | b'/'))
        .collect::<Vec<u8>>();
    let padding = value.bytes().filter(|byte| *byte == b'=').count();
    match symbols.len().rem_euclid(4) {
        1 => Err(base64::DecodeError::InvalidLength(symbols.len())),
        remainder @ (2 | 3) if remainder.saturating_add(padding) < 4 => {
            Err(base64::DecodeError::InvalidPadding)
        }
        _ => PYTHON_BASE64.decode(symbols),
    }
}
