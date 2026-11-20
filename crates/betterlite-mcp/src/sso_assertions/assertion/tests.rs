#![cfg(test)]

use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_auth::jwt_claims::TypedValue;
use chrono::{DateTime, Utc};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tracing::subscriber::DefaultGuard;
use tracing::subscriber::set_default;

use super::{
    AssertionError, SsoIdentityAssertion, UNDECODABLE_WARNING, assertion_expired,
    assertion_from_sso_login, decode_payload, encode_payload,
};

fn json_field(json: &str) -> TypedValue {
    serde_json::from_str(json).unwrap()
}

fn text_field(text: &str) -> TypedValue {
    json_field(&serde_json::to_string(text).unwrap())
}

fn segment(text: &str) -> String {
    URL_SAFE_NO_PAD.encode(text)
}

fn token(header: &str, payload: &str) -> String {
    format!(
        "{}.{}.{}",
        segment(header),
        segment(payload),
        segment("signature")
    )
}

fn jwt(payload: &str) -> String {
    token(r#"{"alg":"RS256","typ":"JWT"}"#, payload)
}

fn moment(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).expect("valid timestamp")
}

fn assertion() -> SsoIdentityAssertion {
    SsoIdentityAssertion {
        id_token: SecretString::from("id-token".to_owned()),
        refresh_token: None,
        issuer: None,
        expires_at: None,
    }
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

fn lossy_text(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
        }
    }
    text
}

#[test]
fn full_claims_fill_every_field() {
    let id_token = jwt(r#"{"exp":1700000000,"iss":"https://idp.example","sub":"user-1"}"#);
    let assertion =
        assertion_from_sso_login(Some(&text_field(&id_token)), Some(&text_field("refresh-1")))
            .unwrap();
    assert_eq!(assertion.id_token.expose_secret(), id_token.as_str());
    assert_eq!(
        assertion.refresh_token.unwrap().expose_secret(),
        "refresh-1"
    );
    assert_eq!(assertion.issuer.as_deref(), Some("https://idp.example"));
    assert_eq!(assertion.expires_at, Some(moment(1_700_000_000)));
}

#[rstest]
#[case::absent(r#"{"sub":"user-1"}"#, None, None)]
#[case::null_claims(r#"{"exp":null,"iss":null}"#, None, None)]
#[case::exp_only(r#"{"exp":1700000000}"#, None, Some(1_700_000_000))]
#[case::iss_only(r#"{"iss":"https://idp.example"}"#, Some("https://idp.example"), None)]
#[case::empty_issuer(r#"{"iss":""}"#, Some(""), None)]
fn absent_or_null_claims_leave_the_field_unset(
    #[case] payload: &str,
    #[case] issuer: Option<&str>,
    #[case] expires_at: Option<i64>,
) {
    let assertion = assertion_from_sso_login(Some(&text_field(&jwt(payload))), None).unwrap();
    assert_eq!(assertion.issuer.as_deref(), issuer);
    assert_eq!(assertion.expires_at, expires_at.map(moment));
}

#[rstest]
#[case::absent(None)]
#[case::null(Some("null"))]
#[case::number(Some("5"))]
#[case::boolean(Some("true"))]
#[case::array(Some("[]"))]
#[case::object(Some(r#"{"token":"x"}"#))]
#[case::empty(Some(r#""""#))]
fn a_missing_or_non_text_id_token_returns_none_without_a_warning(#[case] id_token: Option<&str>) {
    let (captured, _guard) = capture();
    let field = id_token.map(json_field);
    let assertion = assertion_from_sso_login(field.as_ref(), Some(&text_field("refresh-1")));
    assert!(assertion.is_none());
    assert!(captured.text().is_empty());
}

#[rstest]
#[case::absent(None)]
#[case::null(Some("null"))]
#[case::number(Some("7"))]
#[case::boolean(Some("false"))]
#[case::array(Some("[]"))]
#[case::empty(Some(r#""""#))]
fn a_missing_or_non_text_refresh_token_is_dropped(#[case] refresh_token: Option<&str>) {
    let field = refresh_token.map(json_field);
    let assertion = assertion_from_sso_login(
        Some(&text_field(&jwt(r#"{"exp":1700000000}"#))),
        field.as_ref(),
    )
    .unwrap();
    assert!(assertion.refresh_token.is_none());
    assert_eq!(assertion.expires_at, Some(moment(1_700_000_000)));
}

#[rstest]
#[case::not_a_jwt("not-a-jwt")]
#[case::one_segment("aaa")]
#[case::two_segments("aaa.bbb")]
#[case::four_segments("aaa.bbb.ccc.ddd")]
#[case::header_not_base64("###.e30.c2ln")]
#[case::header_not_json("bm90LWpzb24.e30.c2ln")]
#[case::header_not_an_object("W10.e30.c2ln")]
#[case::signature_not_base64("eyJhbGciOiJSUzI1NiJ9.e30.!!!")]
#[case::payload_not_json("eyJhbGciOiJSUzI1NiJ9.bm90LWpzb24.c2ln")]
#[case::payload_not_an_object("eyJhbGciOiJSUzI1NiJ9.WzFd.c2ln")]
fn an_undecodable_token_logs_the_warning_and_returns_none(#[case] id_token: &str) {
    let (captured, _guard) = capture();
    let assertion = assertion_from_sso_login(Some(&text_field(id_token)), None);
    assert!(assertion.is_none());
    assert!(captured.text().contains(UNDECODABLE_WARNING));
}

#[rstest]
#[case::text_exp(r#"{"exp":"1700000000"}"#)]
#[case::boolean_exp(r#"{"exp":true}"#)]
#[case::array_exp(r#"{"exp":[]}"#)]
#[case::number_iss(r#"{"iss":5}"#)]
#[case::object_iss(r#"{"iss":{}}"#)]
#[case::year_too_large(r#"{"exp":253402300800}"#)]
#[case::year_too_small(r#"{"exp":-62135596801}"#)]
#[case::larger_than_a_duration(r#"{"exp":1e300}"#)]
fn unusable_claims_log_the_warning_and_return_none(#[case] payload: &str) {
    let (captured, _guard) = capture();
    let assertion = assertion_from_sso_login(Some(&text_field(&jwt(payload))), None);
    assert!(assertion.is_none());
    assert!(captured.text().contains(UNDECODABLE_WARNING));
}

#[test]
fn a_token_over_the_decoder_limits_logs_the_warning_and_returns_none() {
    let oversized = jwt(&format!(r#"{{"sub":"{}"}}"#, "a".repeat(1_100_000)));
    let mut nested = r#"{"a":"#.repeat(100);
    nested.push('1');
    nested.push_str(&"}".repeat(100));
    let deeply_nested = jwt(&nested);
    let (captured, _guard) = capture();
    let over_the_size = assertion_from_sso_login(Some(&text_field(&oversized)), None);
    let over_the_depth = assertion_from_sso_login(Some(&text_field(&deeply_nested)), None);
    assert!(over_the_size.is_none());
    assert!(over_the_depth.is_none());
    assert_eq!(captured.text().matches(UNDECODABLE_WARNING).count(), 2);
}

#[rstest]
#[case::half_a_second_after_the_epoch(0.5_f64, 0, 500_000_000)]
#[case::half_a_second_before_the_epoch(-0.5_f64, -1, 500_000_000)]
#[case::one_and_a_quarter(1.25_f64, 1, 250_000_000)]
#[case::negative_whole(-2.0_f64, -2, 0)]
fn a_fractional_exp_is_a_nanosecond_precise_instant(
    #[case] exp: f64,
    #[case] secs: i64,
    #[case] nanos: u32,
) {
    let payload = format!(r#"{{"exp":{exp}}}"#);
    let assertion = assertion_from_sso_login(Some(&text_field(&jwt(&payload))), None).unwrap();
    assert_eq!(assertion.expires_at, DateTime::from_timestamp(secs, nanos));
}

#[rstest]
#[case::no_expiry(None, 1_700_000_000, false)]
#[case::one_second_before(Some(1_700_000_001), 1_700_000_000, false)]
#[case::at_the_exact_expiry(Some(1_700_000_000), 1_700_000_000, true)]
#[case::one_second_after(Some(1_699_999_999), 1_700_000_000, true)]
#[case::a_future_now(Some(1_700_000_000), 1_700_000_060, true)]
fn assertion_expired_compares_against_now(
    #[case] expires_at: Option<i64>,
    #[case] now: i64,
    #[case] expected: bool,
) {
    let assertion = SsoIdentityAssertion {
        expires_at: expires_at.map(moment),
        ..assertion()
    };
    assert_eq!(assertion_expired(&assertion, moment(now)), expected);
}

#[test]
fn a_stored_payload_round_trips_every_field() {
    let assertion = SsoIdentityAssertion {
        refresh_token: Some(SecretString::from("refresh-1".to_owned())),
        issuer: Some("https://idp.example".to_owned()),
        expires_at: Some(moment(1_700_000_000)),
        ..assertion()
    };
    let text = encode_payload(&assertion).unwrap();
    assert_eq!(
        text,
        r#"{"id_token":"id-token","refresh_token":"refresh-1","issuer":"https://idp.example","expires_at":"2023-11-14T22:13:20+00:00"}"#
    );
    let decoded = decode_payload(&text).unwrap();
    assert_eq!(decoded.id_token.expose_secret(), "id-token");
    assert_eq!(decoded.refresh_token.unwrap().expose_secret(), "refresh-1");
    assert_eq!(decoded.issuer.as_deref(), Some("https://idp.example"));
    assert_eq!(decoded.expires_at, Some(moment(1_700_000_000)));
}

#[test]
fn unset_fields_stay_out_of_the_stored_payload() {
    let text = encode_payload(&assertion()).unwrap();
    assert_eq!(text, r#"{"id_token":"id-token"}"#);
}

#[rstest]
#[case::naive_utc(
    r#"{"id_token":"id-token","expires_at":"2023-11-14T22:13:20"}"#,
    DateTime::from_timestamp(1_700_000_000, 0)
)]
#[case::explicit_utc(
    r#"{"id_token":"id-token","expires_at":"2023-11-14T22:13:20+00:00"}"#,
    DateTime::from_timestamp(1_700_000_000, 0)
)]
#[case::another_offset(
    r#"{"id_token":"id-token","expires_at":"2023-11-15T03:13:20+05:00"}"#,
    DateTime::from_timestamp(1_700_000_000, 0)
)]
#[case::fractional(
    r#"{"id_token":"id-token","expires_at":"2023-11-14T22:13:20.500+00:00"}"#,
    DateTime::from_timestamp(1_700_000_000, 500_000_000)
)]
#[case::zulu(
    r#"{"id_token":"id-token","expires_at":"2023-11-14T22:13:20Z"}"#,
    DateTime::from_timestamp(1_700_000_000, 0)
)]
fn stored_expires_at_text_reads_as_a_utc_instant(
    #[case] text: &str,
    #[case] expected: Option<DateTime<Utc>>,
) {
    let decoded = decode_payload(text).unwrap();
    assert_eq!(decoded.expires_at, expected);
}

#[test]
fn a_stored_expires_at_outside_the_calendar_years_is_refused() {
    let error = decode_payload(r#"{"id_token":"id-token","expires_at":"+262000-01-01T00:00:00"}"#)
        .unwrap_err();
    assert!(matches!(error, AssertionError::TimestampRange { .. }));
}

#[test]
fn a_stored_payload_with_an_unknown_key_still_reads() {
    let decoded = decode_payload(r#"{"id_token":"id-token","note":"kept"}"#).unwrap();
    assert_eq!(decoded.id_token.expose_secret(), "id-token");
}

#[test]
fn the_stored_read_drops_an_empty_refresh_and_keeps_an_empty_issuer() {
    let decoded =
        decode_payload(r#"{"id_token":"id-token","refresh_token":"","issuer":""}"#).unwrap();
    assert!(decoded.refresh_token.is_none());
    assert_eq!(decoded.issuer.as_deref(), Some(""));
}

#[test]
fn empty_refresh_and_issuer_stay_out_of_the_stored_payload() {
    let assertion = SsoIdentityAssertion {
        refresh_token: Some(SecretString::from(String::new())),
        issuer: Some(String::new()),
        ..assertion()
    };
    assert_eq!(
        encode_payload(&assertion).unwrap(),
        r#"{"id_token":"id-token"}"#
    );
}

#[rstest]
#[case::not_json("not json")]
#[case::not_an_object("[]")]
#[case::missing_id_token(r#"{"refresh_token":"refresh-1"}"#)]
#[case::non_text_id_token(r#"{"id_token":5}"#)]
#[case::non_text_refresh_token(r#"{"id_token":"id-token","refresh_token":5}"#)]
#[case::bad_expires_at(r#"{"id_token":"id-token","expires_at":"tomorrow"}"#)]
#[case::expires_at_out_of_range(
    r#"{"id_token":"id-token","expires_at":"+262000-01-01T00:00:00+00:00"}"#
)]
fn a_malformed_stored_payload_returns_the_error(#[case] text: &str) {
    let error = decode_payload(text).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("the stored SSO identity assertion")
    );
}

#[test]
fn tokens_stay_out_of_debug_output_logs_and_errors() {
    let id_token = jwt(r#"{"exp":1700000000}"#);
    let secret = "super-secret-refresh-value";
    let assertion =
        assertion_from_sso_login(Some(&text_field(&id_token)), Some(&text_field(secret))).unwrap();
    let debug = format!("{assertion:?}");
    assert!(!debug.contains(id_token.as_str()));
    assert!(!debug.contains(secret));

    let (captured, _guard) = capture();
    let offered = format!("live-token-{secret}");
    let refused = assertion_from_sso_login(Some(&text_field(&offered)), Some(&text_field(secret)));
    assert!(refused.is_none());
    assert!(captured.text().contains(UNDECODABLE_WARNING));
    assert!(!captured.text().contains(&offered));
    assert!(!captured.text().contains(secret));

    let payload = format!(r#"{{"id_token":"{id_token}","expires_at":"never"}}"#);
    let error = decode_payload(&payload).unwrap_err();
    assert!(!error.to_string().contains(id_token.as_str()));
}

#[rstest]
fn replays_the_sso_assertion_corpus(
    #[files("fuzz/corpus/sso_assertion_decode/*")] corpus_path: PathBuf,
) {
    let bytes = fs::read(corpus_path).unwrap();
    let text = lossy_text(&bytes);
    drop(assertion_from_sso_login(Some(&text_field(&text)), None));
    drop(decode_payload(&text));
}
