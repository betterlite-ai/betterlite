#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use aws_lc_rs::hmac;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::DateTime;
use rstest::rstest;
use secrecy::SecretString;

use super::{ClaimText, DecodeError, decode_session_cookie};

const MASTER_KEY: &str = "sk-oracle-master";
const NOW: i64 = 1_700_000_000;
const FUTURE_EXPIRY: i64 = 4_102_444_800;
const HEADER: &str = r#"{"alg":"HS256"}"#;
const VALID_PAYLOAD: &str = r#"{"exp":4102444800,"login_method":"sso","user_id":"user-1"}"#;

fn segment(text: &str) -> String {
    URL_SAFE_NO_PAD.encode(text)
}

fn token(header: &str, payload: &str) -> Vec<u8> {
    let signing_input = format!("{}.{}", segment(header), segment(payload));
    let signature = hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, MASTER_KEY.as_bytes()),
        signing_input.as_bytes(),
    );
    format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature.as_ref())
    )
    .into_bytes()
}

#[test]
fn a_signed_ui_cookie_decodes_its_claims() {
    let cookie = decode_session_cookie(&token(HEADER, VALID_PAYLOAD)).unwrap();
    assert_eq!(cookie.claims.exp, FUTURE_EXPIRY);
    assert_eq!(cookie.claims.iat, None);
    assert_eq!(cookie.claims.nbf, None);
    assert_eq!(cookie.claims.user_id, ClaimText::Text("user-1".to_owned()));
    assert_eq!(
        cookie.claims.login_method,
        ClaimText::Text("sso".to_owned())
    );
    assert!(cookie.claims.session_key.is_none());
    assert_eq!(cookie.claims.token_type, ClaimText::Absent);
    assert!(cookie.signature_matches(&SecretString::from(MASTER_KEY)));
    assert!(!cookie.signature_matches(&SecretString::from("sk-other-master")));
    assert!(cookie.claims_are_current(DateTime::from_timestamp(NOW, 0).unwrap()));
}

#[test]
fn expiry_is_refused_at_the_current_second() {
    let cookie = decode_session_cookie(&token(HEADER, r#"{"exp":1700000000}"#)).unwrap();
    assert!(!cookie.claims_are_current(DateTime::from_timestamp(NOW, 0).unwrap()));
    assert!(cookie.claims_are_current(DateTime::from_timestamp(NOW.saturating_sub(1), 0).unwrap()));
}

#[test]
fn a_future_iat_or_nbf_is_refused() {
    let now = DateTime::from_timestamp(NOW, 0).unwrap();
    let issued_at =
        decode_session_cookie(&token(HEADER, r#"{"exp":4102444800,"iat":1700000001}"#)).unwrap();
    assert!(!issued_at.claims_are_current(now));
    let not_before =
        decode_session_cookie(&token(HEADER, r#"{"exp":4102444800,"nbf":1700000001}"#)).unwrap();
    assert!(!not_before.claims_are_current(now));
    let boundary = decode_session_cookie(&token(
        HEADER,
        r#"{"exp":4102444800,"iat":1700000000,"nbf":1700000000}"#,
    ))
    .unwrap();
    assert!(boundary.claims_are_current(now));
}

#[test]
fn time_claims_beyond_i64_keep_python_comparisons() {
    let now = DateTime::from_timestamp(NOW, 0).unwrap();
    let far_future =
        decode_session_cookie(&token(HEADER, r#"{"exp":9223372036854775808}"#)).unwrap();
    assert!(far_future.claims_are_current(now));
    let far_past =
        decode_session_cookie(&token(HEADER, r#"{"exp":-9223372036854775809}"#)).unwrap();
    assert!(!far_past.claims_are_current(now));
    let later = decode_session_cookie(&token(
        HEADER,
        r#"{"exp":9223372036854775808,"iat":9223372036854775809}"#,
    ))
    .unwrap();
    assert!(!later.claims_are_current(now));
    let very_large = decode_session_cookie(&token(
        HEADER,
        r#"{"exp":123456789012345678901234567890123456789012345678901234567890}"#,
    ))
    .unwrap();
    assert!(very_large.claims_are_current(now));
}

#[test]
fn absent_null_and_non_text_claims_are_told_apart() {
    let payload = r#"{"exp":1,"user_id":null,"login_method":7,"key":true}"#;
    let cookie = decode_session_cookie(&token(HEADER, payload)).unwrap();
    assert_eq!(cookie.claims.user_id, ClaimText::Absent);
    assert_eq!(cookie.claims.login_method, ClaimText::Other);
    assert!(cookie.claims.session_key.is_none());
    assert_eq!(cookie.claims.token_type, ClaimText::Absent);
}

#[rstest]
#[case::audience_null(r#"{"exp":1,"aud":null}"#)]
#[case::audience_empty_text(r#"{"exp":1,"aud":""}"#)]
#[case::audience_empty_list(r#"{"exp":1,"aud":[]}"#)]
fn an_empty_audience_is_accepted(#[case] payload: &str) {
    let cookie = decode_session_cookie(&token(HEADER, payload)).unwrap();
    assert_eq!(cookie.claims.exp, 1);
}

#[rstest]
#[case::missing_exp(
    r#"{"login_method":"sso","user_id":"user-1"}"#,
    DecodeError::MissingExpiry
)]
#[case::null_exp(r#"{"exp":null}"#, DecodeError::Expiry)]
#[case::text_exp(r#"{"exp":"4102444800"}"#, DecodeError::Expiry)]
#[case::float_exp(r#"{"exp":1.5}"#, DecodeError::Expiry)]
#[case::text_iat(r#"{"exp":1,"iat":"now"}"#, DecodeError::IssuedAt)]
#[case::bool_nbf(r#"{"exp":1,"nbf":false}"#, DecodeError::NotBefore)]
#[case::audience_text(r#"{"exp":1,"aud":"https://issuer.example"}"#, DecodeError::Audience)]
#[case::audience_list(r#"{"exp":1,"aud":["https://issuer.example"]}"#, DecodeError::Audience)]
#[case::number_subject(r#"{"exp":1,"sub":1}"#, DecodeError::Subject)]
#[case::null_subject(r#"{"exp":1,"sub":null}"#, DecodeError::Subject)]
#[case::number_token_id(r#"{"exp":1,"jti":1}"#, DecodeError::TokenId)]
fn claim_errors_are_refused(#[case] payload: &str, #[case] expected: DecodeError) {
    assert_eq!(
        decode_session_cookie(&token(HEADER, payload)).unwrap_err(),
        expected
    );
}

#[rstest]
#[case::algorithm_hs512(r#"{"alg":"HS512"}"#, DecodeError::Algorithm)]
#[case::algorithm_none(r#"{"alg":"none"}"#, DecodeError::Algorithm)]
#[case::algorithm_missing("{}", DecodeError::Algorithm)]
#[case::algorithm_number(r#"{"alg":256}"#, DecodeError::Algorithm)]
#[case::key_id_number(r#"{"alg":"HS256","kid":1}"#, DecodeError::KeyId)]
#[case::crit_text(r#"{"alg":"HS256","crit":"b64"}"#, DecodeError::Critical)]
#[case::crit_empty(r#"{"alg":"HS256","crit":[]}"#, DecodeError::Critical)]
#[case::crit_unsupported(r#"{"alg":"HS256","crit":["kid"]}"#, DecodeError::Critical)]
#[case::crit_missing_from_headers(r#"{"alg":"HS256","crit":["b64"]}"#, DecodeError::Critical)]
#[case::crit_number(r#"{"alg":"HS256","crit":[1]}"#, DecodeError::Critical)]
#[case::detached_payload(
    r#"{"alg":"HS256","crit":["b64"],"b64":false}"#,
    DecodeError::DetachedPayload
)]
fn header_errors_are_refused(#[case] header: &str, #[case] expected: DecodeError) {
    assert_eq!(
        decode_session_cookie(&token(header, VALID_PAYLOAD)).unwrap_err(),
        expected
    );
}

#[test]
fn a_critical_b64_header_is_accepted() {
    let header = r#"{"alg":"HS256","crit":["b64"],"b64":true}"#;
    let cookie = decode_session_cookie(&token(header, VALID_PAYLOAD)).unwrap();
    assert_eq!(cookie.claims.exp, FUTURE_EXPIRY);
}

#[test]
fn byte_input_that_is_not_text_or_a_compact_jwt_is_refused() {
    assert_eq!(
        decode_session_cookie(&[0xff, 0xfe]).unwrap_err(),
        DecodeError::NotText
    );
    assert_eq!(
        decode_session_cookie(b"").unwrap_err(),
        DecodeError::NotCompact
    );
    assert_eq!(
        decode_session_cookie(b"a.b").unwrap_err(),
        DecodeError::NotCompact
    );
}

#[test]
fn each_segment_must_be_canonical_base64url() {
    let header = segment(HEADER);
    let payload = segment(VALID_PAYLOAD);
    let signature = segment("signature");
    assert_eq!(
        decode_session_cookie(format!("!!!!.{payload}.{signature}").as_bytes()).unwrap_err(),
        DecodeError::Header
    );
    assert_eq!(
        decode_session_cookie(format!("{header}.!!!!.{signature}").as_bytes()).unwrap_err(),
        DecodeError::Payload
    );
    assert_eq!(
        decode_session_cookie(format!("{header}.{payload}.!!!!").as_bytes()).unwrap_err(),
        DecodeError::Signature
    );
}

#[test]
fn header_and_payload_must_be_json_objects() {
    let header = segment(HEADER);
    let payload = segment(VALID_PAYLOAD);
    let signature = segment("signature");
    assert_eq!(
        decode_session_cookie(format!("{}.{payload}.{signature}", segment("[]")).as_bytes())
            .unwrap_err(),
        DecodeError::Header
    );
    assert_eq!(
        decode_session_cookie(format!("{}.{payload}.{signature}", segment("nope")).as_bytes())
            .unwrap_err(),
        DecodeError::Header
    );
    assert_eq!(
        decode_session_cookie(format!("{header}.{}.{signature}", segment("[]")).as_bytes())
            .unwrap_err(),
        DecodeError::Payload
    );
    assert_eq!(
        decode_session_cookie(format!("{header}.{}.{signature}", segment("nope")).as_bytes())
            .unwrap_err(),
        DecodeError::Payload
    );
}

#[test]
fn json_that_python_accepts_but_standard_json_refuses() {
    let nan = token(
        HEADER,
        r#"{"exp":1,"login_method":"sso","user_id":"user-1","x":NaN}"#,
    );
    assert_eq!(
        decode_session_cookie(&nan).unwrap_err(),
        DecodeError::Payload
    );
    let bom = token("\u{feff}{\"alg\":\"HS256\"}", VALID_PAYLOAD);
    assert_eq!(
        decode_session_cookie(&bom).unwrap_err(),
        DecodeError::Header
    );
}

#[test]
fn segment_padding_follows_the_compact_encoding_rules() {
    let compact = String::from_utf8(token(HEADER, VALID_PAYLOAD)).unwrap();
    assert_eq!(
        decode_session_cookie(format!("{compact}=").as_bytes())
            .unwrap()
            .claims
            .exp,
        FUTURE_EXPIRY
    );
    assert_eq!(
        decode_session_cookie(format!("{compact}==").as_bytes()).unwrap_err(),
        DecodeError::Signature
    );
    let header = segment(HEADER);
    let payload = segment(VALID_PAYLOAD);
    assert_eq!(
        decode_session_cookie(format!("{header}.{payload}.aQ==").as_bytes())
            .unwrap()
            .claims
            .exp,
        FUTURE_EXPIRY
    );
    assert_eq!(
        decode_session_cookie(format!("{header}====.{payload}.AAAA").as_bytes()).unwrap_err(),
        DecodeError::Header
    );
    let padded_payload = format!("{}=", segment(r#"{"exp":1}"#));
    assert_eq!(
        decode_session_cookie(format!("{header}.{padded_payload}.AAAA").as_bytes()).unwrap_err(),
        DecodeError::Payload
    );
    assert_eq!(
        decode_session_cookie(format!("{header}.{}.A", segment(VALID_PAYLOAD)).as_bytes())
            .unwrap_err(),
        DecodeError::Signature
    );
}

#[rstest]
fn replays_the_decode_session_cookie_corpus(
    #[files("fuzz/corpus/decode_session_cookie/*")] path: PathBuf,
) {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap()
        .to_owned();
    let bytes = fs::read(path).unwrap();
    let decoded = decode_session_cookie(&bytes).is_ok();
    assert_eq!(decoded, name.starts_with("ok-"), "seed {name}");
}
