#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::SecretString;

use crate::endpoint_target::target::AuthHeader;
use crate::gemini_http::error::GeminiHttpError;
use crate::gemini_http::headers::{psc_endpoint_format, request_headers};

use super::support::{GEMINI_KEY, TOKEN, exposed, object};

fn caller(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn auth_map(pairs: &[(&str, &str)]) -> AuthHeader {
    let headers = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), SecretString::from((*value).to_owned())))
        .collect();
    AuthHeader::Headers(headers)
}

#[test]
fn headers_default_to_the_content_type() {
    let headers = request_headers(None, &BTreeMap::new());
    let expected = BTreeMap::from([(
        String::from("Content-Type"),
        String::from("application/json"),
    )]);
    assert_eq!(exposed(&headers), expected);
}

#[test]
fn headers_a_token_becomes_a_bearer() {
    let auth = AuthHeader::Token(SecretString::from(TOKEN.to_owned()));
    let headers = request_headers(Some(&auth), &BTreeMap::new());
    let expected = BTreeMap::from([
        (
            String::from("Content-Type"),
            String::from("application/json"),
        ),
        (String::from("Authorization"), format!("Bearer {TOKEN}")),
    ]);
    assert_eq!(exposed(&headers), expected);
}

#[test]
fn headers_an_auth_map_lands_over_the_content_type() {
    let auth = auth_map(&[("x-goog-api-key", GEMINI_KEY)]);
    let headers = request_headers(Some(&auth), &BTreeMap::new());
    let expected = BTreeMap::from([
        (
            String::from("Content-Type"),
            String::from("application/json"),
        ),
        (String::from("x-goog-api-key"), String::from(GEMINI_KEY)),
    ]);
    assert_eq!(exposed(&headers), expected);
}

#[test]
fn headers_an_auth_map_may_replace_the_content_type() {
    let auth = auth_map(&[("Content-Type", "application/x-www-form-urlencoded")]);
    let headers = request_headers(Some(&auth), &BTreeMap::new());
    assert_eq!(
        exposed(&headers).get("Content-Type").map(String::as_str),
        Some("application/x-www-form-urlencoded")
    );
}

#[test]
fn headers_the_caller_wins() {
    let auth = AuthHeader::Token(SecretString::from(TOKEN.to_owned()));
    let extra = caller(&[
        ("Authorization", "Bearer caller-key"),
        ("Content-Type", "text/plain"),
        ("x-goog-custom", "kept"),
    ]);
    let headers = request_headers(Some(&auth), &extra);
    let expected = BTreeMap::from([
        (
            String::from("Authorization"),
            String::from("Bearer caller-key"),
        ),
        (String::from("Content-Type"), String::from("text/plain")),
        (String::from("x-goog-custom"), String::from("kept")),
    ]);
    assert_eq!(exposed(&headers), expected);
}

#[test]
fn headers_the_caller_wins_over_an_auth_map() {
    let auth = auth_map(&[("x-goog-api-key", GEMINI_KEY), ("x-goog-auth", "auth")]);
    let extra = caller(&[("x-goog-api-key", "caller-key"), ("x-goog-auth", "caller")]);
    let headers = request_headers(Some(&auth), &extra);
    let expected = BTreeMap::from([
        (
            String::from("Content-Type"),
            String::from("application/json"),
        ),
        (String::from("x-goog-api-key"), String::from("caller-key")),
        (String::from("x-goog-auth"), String::from("caller")),
    ]);
    assert_eq!(exposed(&headers), expected);
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"use_psc_endpoint_format":null}"#)]
fn headers_the_psc_flag_defaults_false(#[case] params: &str) {
    assert!(!psc_endpoint_format(&object(params)).unwrap());
}

#[rstest]
#[case::on(r#"{"use_psc_endpoint_format":true}"#, true)]
#[case::off(r#"{"use_psc_endpoint_format":false}"#, false)]
fn headers_the_psc_flag_reads_the_boolean(#[case] params: &str, #[case] expected: bool) {
    assert_eq!(psc_endpoint_format(&object(params)).unwrap(), expected);
}

#[rstest]
#[case::text(r#"{"use_psc_endpoint_format":"true"}"#)]
#[case::number(r#"{"use_psc_endpoint_format":1}"#)]
#[case::array(r#"{"use_psc_endpoint_format":[]}"#)]
fn headers_the_psc_flag_refuses_other_values(#[case] params: &str) {
    let error = psc_endpoint_format(&object(params)).unwrap_err();
    assert!(matches!(
        error,
        GeminiHttpError::Flag {
            field: "use_psc_endpoint_format"
        }
    ));
    assert_eq!(
        error.to_string(),
        "the use_psc_endpoint_format value must be true or false"
    );
}
