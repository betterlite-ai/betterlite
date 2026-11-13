#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{TokenFlow, decode_wxo_token_response};

const NOW: i64 = 1_700_000_000;

fn decode(body: &str, flow: TokenFlow) -> (String, i64) {
    let grant = decode_wxo_token_response(body.as_bytes(), flow, NOW).unwrap();
    (
        grant.token.expose_secret().to_owned(),
        grant.lifetime_seconds,
    )
}

#[rstest]
#[case::duration(r#"{"access_token": "t", "expires_in": 1200}"#, 1200)]
#[case::default_when_absent(r#"{"access_token": "t"}"#, 3600)]
#[case::default_when_null(r#"{"access_token": "t", "expires_in": null}"#, 3600)]
#[case::zero_is_used_as_given(r#"{"access_token": "t", "expires_in": 0}"#, 0)]
#[case::negative_is_used_as_given(r#"{"access_token": "t", "expires_in": -5}"#, -5)]
fn ibm_cloud_expires_in_is_a_duration(#[case] body: &str, #[case] lifetime: i64) {
    assert_eq!(
        decode(body, TokenFlow::IbmCloud),
        ("t".to_owned(), lifetime)
    );
}

#[rstest]
#[case::future_epoch(r#"{"token": "t", "expiration": 1700000900}"#, 900)]
#[case::fraction_truncates(r#"{"token": "t", "expiration": 1700000900.9}"#, 900)]
#[case::now(r#"{"token": "t", "expiration": 1700000000}"#, 0)]
#[case::past_is_zero(r#"{"token": "t", "expiration": 1699999000}"#, 0)]
#[case::default_when_absent(r#"{"token": "t"}"#, 3600)]
#[case::default_when_null(r#"{"token": "t", "expiration": null}"#, 3600)]
#[case::huge_is_clamped(r#"{"token": "t", "expiration": 1e300}"#, i64::MAX - NOW)]
#[case::huge_negative_is_zero(r#"{"token": "t", "expiration": -1e300}"#, 0)]
fn cp4d_expiration_is_an_absolute_epoch(#[case] body: &str, #[case] lifetime: i64) {
    assert_eq!(decode(body, TokenFlow::Cp4d), ("t".to_owned(), lifetime));
}

#[rstest]
#[case::ibm_without_token(TokenFlow::IbmCloud, r#"{"token": "t"}"#)]
#[case::cp4d_without_token(TokenFlow::Cp4d, r#"{"access_token": "t"}"#)]
#[case::ibm_token_not_text(TokenFlow::IbmCloud, r#"{"access_token": 5}"#)]
#[case::ibm_float_duration(TokenFlow::IbmCloud, r#"{"access_token": "t", "expires_in": 1.5}"#)]
#[case::cp4d_expiration_text(TokenFlow::Cp4d, r#"{"token": "t", "expiration": "soon"}"#)]
#[case::array(TokenFlow::Cp4d, "[]")]
#[case::truncated(TokenFlow::IbmCloud, r#"{"access_token": "t", "expi"#)]
#[case::empty(TokenFlow::Cp4d, "")]
fn malformed_answers_are_errors_not_panics(#[case] flow: TokenFlow, #[case] body: &str) {
    let error = decode_wxo_token_response(body.as_bytes(), flow, NOW).unwrap_err();
    assert_eq!(error.to_string(), "token response is not the expected JSON");
}

#[rstest]
fn replays_the_watsonx_token_response_corpus(
    #[files("fuzz/corpus/watsonx_token_response/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(decode_wxo_token_response(&bytes, TokenFlow::IbmCloud, NOW));
    if let Ok(grant) = decode_wxo_token_response(&bytes, TokenFlow::Cp4d, NOW) {
        assert!(grant.lifetime_seconds >= 0);
    }
}
