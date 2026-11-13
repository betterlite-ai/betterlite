use base64::Engine as _;
use base64::alphabet::URL_SAFE;
use base64::engine::DecodePaddingMode;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use num_traits::ToPrimitive as _;
use serde::Deserialize;

pub const EXPIRY_MARGIN_SECONDS: i64 = 60;

// real OIDC payloads are a few KiB; the cap keeps a hostile token from being decoded at any size
const MAX_PAYLOAD_BYTES: usize = 0x10_0000;

// padding is optional in a JWT segment, and trailing bits are not checked
const PAYLOAD_DECODER: GeneralPurpose = GeneralPurpose::new(
    &URL_SAFE,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

#[derive(Debug, Deserialize)]
pub struct OidcTokenClaims {
    pub exp: Option<f64>,
}

#[must_use]
pub fn oidc_token_cache_ttl(oidc_token: &str, max_ttl: i64, now_unix_seconds: i64) -> i64 {
    let mut segments = oidc_token.split('.');
    let (Some(_header), Some(payload), Some(_signature), None) = (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) else {
        return max_ttl;
    };
    if payload.len() > MAX_PAYLOAD_BYTES {
        return max_ttl;
    }
    let Ok(decoded) = PAYLOAD_DECODER.decode(payload) else {
        return max_ttl;
    };
    // serde would read a JSON array as the struct's fields in order
    if decoded.trim_ascii_start().first() != Some(&b'{') {
        return max_ttl;
    }
    let Ok(OidcTokenClaims { exp: Some(exp) }) =
        serde_json::from_slice::<OidcTokenClaims>(&decoded)
    else {
        return max_ttl;
    };
    let expires_at = match exp.to_i64() {
        Some(seconds) => seconds,
        None if exp.is_sign_positive() => return max_ttl,
        None => i64::MIN,
    };
    max_ttl.min(
        expires_at
            .saturating_sub(now_unix_seconds)
            .saturating_sub(EXPIRY_MARGIN_SECONDS),
    )
}
