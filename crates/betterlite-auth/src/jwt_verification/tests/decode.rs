use std::fs;
use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_config::sections::jwt_settings::Settings as JwtAuthSettings;
use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::Number;

use crate::jwt_verification::decode::{
    DecodeError, JwtHeader, MAX_SIGNED_JWT_BYTES, MAX_SIGNED_JWT_DEPTH, decode_signed_jwt,
};
use crate::jwt_verification::errors::{VerificationError, into_proxy_exception};

use super::fixtures;
use super::support::{ScriptedSource, claim_fields, harness, parse_json, settings, text_claim};

#[tokio::test]
async fn signed_token_byte_depth_limits_return_401() {
    let harness = harness(settings(
        Some("litellm-proxy"),
        None,
        JwtAuthSettings::default(),
    ));
    let source = ScriptedSource::with_keys(fixtures::RSA_JWK);
    let mut oversized = "a".repeat(MAX_SIGNED_JWT_BYTES);
    oversized.push('a');
    let oversize_error = harness
        .verifier
        .auth_jwt(&oversized, &source)
        .await
        .unwrap_err();
    let oversize_exception = into_proxy_exception(&oversize_error);
    assert_eq!(oversize_exception.code, "401");
    assert_eq!(oversize_exception.error_type, "auth_error");
    assert_eq!(
        oversize_exception.message,
        "Authentication Error, the signed token is over the 1048576-byte limit"
    );
    assert!(source.calls().is_empty());
    byte_limit_boundaries();
    depth_limit_boundaries();
}

fn byte_limit_boundaries() {
    let exact = vec![b'a'; MAX_SIGNED_JWT_BYTES];
    let exact_error = decode_signed_jwt(&exact).unwrap_err();
    assert_eq!(
        exact_error.to_string(),
        "the signed JWT input has fewer than three segments"
    );
    let mut past = vec![b'a'; MAX_SIGNED_JWT_BYTES];
    past.push(b'a');
    let past_error = decode_signed_jwt(&past).unwrap_err();
    assert_eq!(
        past_error.to_string(),
        format!(
            "the signed JWT input is {past_len} bytes, over the {MAX_SIGNED_JWT_BYTES}-byte limit",
            past_len = past.len()
        )
    );
}

fn depth_limit_boundaries() {
    let depth = 65_usize;
    let deep = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
    let deep_error = decode_signed_jwt(deep.as_bytes()).unwrap_err();
    assert_eq!(
        deep_error.to_string(),
        format!("the signed JWT input nests deeper than {MAX_SIGNED_JWT_DEPTH} JSON levels")
    );
    let deep_exception = into_proxy_exception(&VerificationError::from(deep_error));
    assert_eq!(deep_exception.code, "401");
    assert_eq!(
        deep_exception.message,
        "Authentication Error, the signed token nests deeper than 64 JSON levels"
    );

    let shallow = format!("{}{}", "[".repeat(64_usize), "]".repeat(64_usize));
    let shallow_error = decode_signed_jwt(shallow.as_bytes()).unwrap_err();
    assert!(matches!(shallow_error, DecodeError::NotDocument));
}

#[test]
fn decodes_jwk_and_compact_payload() {
    let jwk = decode_signed_jwt(fixtures::RSA_JWK.as_bytes()).unwrap();
    let jwk_fields = claim_fields(&jwk);
    assert_eq!(text_claim(&jwk_fields, "kty"), Some("RSA"));
    assert_eq!(jwk_fields.get("kid"), Some(&parse_json("\"rsa-key\"")));
    let payload = decode_signed_jwt(fixtures::TOKEN_RS256.as_bytes()).unwrap();
    let payload_fields = claim_fields(&payload);
    assert_eq!(text_claim(&payload_fields, "sub"), Some("user-1"));
    assert_eq!(
        payload_fields.get("exp"),
        Some(&Json::Number(Number::from(fixtures::FUTURE_EXP)))
    );
    assert_eq!(
        decode_signed_jwt(b"").unwrap_err().to_string(),
        "the signed JWT input has fewer than three segments"
    );
}

#[rstest]
fn replays_the_jwt_verification_corpus(#[files("fuzz/corpus/jwt_verification/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(decode_signed_jwt(&bytes));
}

fn encoded(text: &str) -> String {
    URL_SAFE_NO_PAD.encode(text.as_bytes())
}

fn header_of(header: &str, payload_segment: &str) -> Result<JwtHeader, DecodeError> {
    JwtHeader::parse(&format!("{}.{}.AAAA", encoded(header), payload_segment))
}

#[test]
fn header_byte_limit_is_inclusive_of_the_limit() {
    let exact = "a".repeat(MAX_SIGNED_JWT_BYTES);
    assert_eq!(
        JwtHeader::parse(&exact).unwrap_err().to_string(),
        "the signed JWT input has fewer than three segments"
    );
    let mut past = exact;
    past.push('a');
    assert_eq!(
        JwtHeader::parse(&past).unwrap_err().to_string(),
        format!(
            "the signed JWT input is {len} bytes, over the {MAX_SIGNED_JWT_BYTES}-byte limit",
            len = past.len()
        )
    );
}

#[test]
fn detached_payload_requires_an_empty_segment() {
    let valid = header_of(r#"{"alg":"RS256","b64":false,"crit":["b64"]}"#, "").unwrap();
    assert_eq!(valid.algorithm.as_deref(), Some("RS256"));
    assert!(matches!(
        header_of(r#"{"alg":"RS256","b64":false}"#, &encoded("{}")),
        Err(DecodeError::Header)
    ));
}

#[test]
fn critical_headers_name_only_a_present_b64_parameter() {
    assert!(matches!(
        header_of(r#"{"alg":"RS256","crit":["b64"]}"#, &encoded("{}")),
        Err(DecodeError::Header)
    ));
    assert!(matches!(
        header_of(r#"{"alg":"RS256","b64":false,"crit":["exp"]}"#, ""),
        Err(DecodeError::Header)
    ));
    assert!(matches!(
        header_of(r#"{"alg":"RS256","crit":"b64"}"#, &encoded("{}")),
        Err(DecodeError::Header)
    ));
    assert!(matches!(
        header_of(r#"{"alg":"RS256","b64":false,"crit":[]}"#, ""),
        Err(DecodeError::Header)
    ));
    assert!(matches!(
        header_of(r#"{"alg":"RS256","b64":false,"crit":[1]}"#, ""),
        Err(DecodeError::Header)
    ));
}

#[test]
fn a_present_null_kid_or_crit_is_rejected() {
    // pyjwt reads a present null as a value: a null kid is not a string, and a null crit is not a list
    assert!(matches!(
        header_of(r#"{"alg":"RS256","kid":null}"#, &encoded("{}")),
        Err(DecodeError::Header)
    ));
    assert!(matches!(
        header_of(r#"{"alg":"RS256","kid":7}"#, &encoded("{}")),
        Err(DecodeError::Header)
    ));
    assert!(matches!(
        header_of(r#"{"alg":"RS256","crit":null}"#, &encoded("{}")),
        Err(DecodeError::Header)
    ));
    // a present b64 satisfies pyjwt's crit check even when its value is null
    let accepted = header_of(
        r#"{"alg":"RS256","b64":null,"crit":["b64"]}"#,
        &encoded("{}"),
    )
    .unwrap();
    assert_eq!(accepted.algorithm.as_deref(), Some("RS256"));
}

#[test]
fn an_invalid_payload_segment_fails_the_header_parse() {
    let error = header_of(r#"{"alg":"RS256"}"#, "!!!").unwrap_err();
    assert!(matches!(error, DecodeError::Claims(_)));
}
