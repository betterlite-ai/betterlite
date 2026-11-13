use base64::Engine as _;
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};

use crate::oidc::token_ttl::oidc_token_cache_ttl;

const NOW: i64 = 1_700_000_000;
const MAX_TTL: i64 = 3600;

fn token_with_payload(payload: &[u8]) -> String {
    format!("e30.{}.c2ln", URL_SAFE_NO_PAD.encode(payload))
}

fn token_expiring_at(exp: &str) -> String {
    token_with_payload(format!(r#"{{"exp": {exp}}}"#).as_bytes())
}

#[rstest::rstest]
#[case::far_future_is_capped_by_max_ttl("1700100000", MAX_TTL)]
#[case::soon_is_capped_by_expiry_minus_margin("1700000300", 240)]
#[case::fraction_truncates("1700000300.9", 240)]
#[case::inside_the_margin_is_zero("1700000060", 0)]
#[case::already_expired_is_negative("1699999500", -560)]
#[case::exactly_max_plus_margin("1700003660", MAX_TTL)]
fn expiry_caps_the_ttl(#[case] exp: &str, #[case] expected: i64) {
    assert_eq!(
        oidc_token_cache_ttl(&token_expiring_at(exp), MAX_TTL, NOW),
        expected
    );
}

#[test]
fn padded_payload_decodes_like_unpadded() {
    let payload = format!(r#"{{"exp": {}}}"#, NOW + 300);
    let token = format!("e30.{}.c2ln", URL_SAFE.encode(payload));
    assert_eq!(oidc_token_cache_ttl(&token, MAX_TTL, NOW), 240);
}

#[test]
fn urlsafe_alphabet_is_decoded() {
    let payload = format!(r#"{{"exp": {}, "x": "??>>"}}"#, NOW + 300);
    let encoded = URL_SAFE_NO_PAD.encode(&payload);
    assert!(encoded.contains('_') || encoded.contains('-'));
    let token = format!("e30.{encoded}.c2ln");
    assert_eq!(oidc_token_cache_ttl(&token, MAX_TTL, NOW), 240);
}

#[rstest::rstest]
#[case::two_segments(String::from("e30.e30"))]
#[case::four_segments(format!("e30.{}.c2ln.extra", URL_SAFE_NO_PAD.encode(r#"{"exp": 1}"#)))]
#[case::no_dots(String::from("opaque-token"))]
#[case::empty(String::new())]
#[case::empty_payload(String::from("e30..c2ln"))]
#[case::not_base64(String::from("e30.!!!.c2ln"))]
#[case::impossible_length(String::from("e30.abcde.c2ln"))]
#[case::not_json(token_with_payload(b"not json"))]
#[case::json_array(token_with_payload(b"[1700000001]"))]
#[case::json_string(token_with_payload(br#""exp""#))]
#[case::no_exp(token_with_payload(br#"{"sub": "x"}"#))]
#[case::null_exp(token_with_payload(br#"{"exp": null}"#))]
#[case::string_exp(token_with_payload(br#"{"exp": "1700000300"}"#))]
#[case::exp_out_of_float_range(token_with_payload(br#"{"exp": 1e999}"#))]
#[case::invalid_utf8(token_with_payload(&[0xff, 0xfe, 0x7b]))]
#[case::trailing_bytes(token_with_payload(br#"{"exp": 1700000300} x"#))]
#[case::exp_beyond_i64(token_with_payload(br#"{"exp": 1e30}"#))]
fn unreadable_expiry_falls_back_to_max_ttl(#[case] token: String) {
    assert_eq!(oidc_token_cache_ttl(&token, MAX_TTL, NOW), MAX_TTL);
}

#[test]
fn an_exp_far_in_the_past_stays_non_positive() {
    let token = token_with_payload(br#"{"exp": -1e30}"#);
    assert!(oidc_token_cache_ttl(&token, MAX_TTL, NOW) < 0);
}

#[test]
fn a_negative_fraction_truncates_toward_zero() {
    let token = token_with_payload(br#"{"exp": -0.5}"#);
    assert_eq!(oidc_token_cache_ttl(&token, MAX_TTL, 0), -60);
}

#[test]
fn extreme_clock_and_ttl_values_do_not_overflow() {
    let token = token_with_payload(br#"{"exp": -1e18}"#);
    assert_eq!(oidc_token_cache_ttl(&token, MAX_TTL, i64::MAX), i64::MIN);
    assert_eq!(oidc_token_cache_ttl(&token, i64::MIN, 0), i64::MIN);
}

#[test]
fn a_payload_over_the_size_cap_is_not_decoded() {
    let padding = "a".repeat(0x10_0000);
    let payload = format!(r#"{{"exp": {}, "pad": "{padding}"}}"#, NOW + 300);
    assert_eq!(
        oidc_token_cache_ttl(&token_with_payload(payload.as_bytes()), MAX_TTL, NOW),
        MAX_TTL
    );
}

#[test]
fn deeply_nested_json_is_rejected_without_overflowing_the_stack() {
    let nested = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    assert_eq!(
        oidc_token_cache_ttl(&token_with_payload(nested.as_bytes()), MAX_TTL, NOW),
        MAX_TTL
    );
}
