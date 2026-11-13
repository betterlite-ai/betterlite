#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::SecretString;
use tokio::runtime::Builder;

use super::error::FetchError;
use super::{HttpFetch, SessionFetch as _, classify, session_url};
use crate::statusline::credentials::Credentials;
use crate::statusline::session::Session;

const VALID_BODY: &str = r#"{"router_name":"auto","last_model":"gpt-5","spend":1.5}"#;

#[rstest]
#[case::ok(200, VALID_BODY, true, true)]
#[case::last_success_status(299, VALID_BODY, true, true)]
#[case::session_that_fails_validation(200, r#"{"router_name":"auto"}"#, false, false)]
#[case::empty_body(200, "", false, false)]
#[case::not_json(200, "<html>", false, false)]
#[case::below_success(199, VALID_BODY, false, false)]
#[case::redirect(300, VALID_BODY, false, false)]
#[case::last_redirect(399, VALID_BODY, false, false)]
#[case::first_client_error(400, VALID_BODY, false, true)]
#[case::not_found(404, "", false, true)]
#[case::last_client_error(499, VALID_BODY, false, true)]
#[case::server_error(500, VALID_BODY, false, false)]
#[case::unavailable(503, "", false, false)]
fn a_status_decides_the_session_and_whether_it_is_final(
    #[case] status: u16,
    #[case] body: &str,
    #[case] has_session: bool,
    #[case] definitive: bool,
) {
    let fetched = classify(status, body.as_bytes());

    assert_eq!(fetched.session.is_some(), has_session);
    assert_eq!(fetched.definitive, definitive);
}

#[test]
fn a_valid_answer_carries_its_session() {
    let fetched = classify(200, VALID_BODY.as_bytes());

    assert_eq!(fetched.session, Session::from_json(VALID_BODY.as_bytes()));
    assert_eq!(fetched.session.unwrap().router_name, "auto");
}

#[rstest]
#[case::plain(
    "http://proxy:4000",
    "abc",
    "http://proxy:4000/auto_router/session?session_id=abc"
)]
#[case::path_prefix(
    "http://proxy/prefix",
    "s",
    "http://proxy/prefix/auto_router/session?session_id=s"
)]
#[case::space(
    "http://proxy",
    "a b",
    "http://proxy/auto_router/session?session_id=a+b"
)]
#[case::reserved_characters(
    "http://proxy",
    "a&b=c#d",
    "http://proxy/auto_router/session?session_id=a%26b%3Dc%23d"
)]
#[case::non_ascii(
    "http://proxy",
    "é",
    "http://proxy/auto_router/session?session_id=%C3%A9"
)]
#[case::empty_session_id("http://proxy", "", "http://proxy/auto_router/session?session_id=")]
fn the_session_url_carries_an_encoded_session_id(
    #[case] base: &str,
    #[case] session_id: &str,
    #[case] expected: &str,
) {
    assert_eq!(session_url(base, session_id).unwrap().as_str(), expected);
}

#[rstest]
#[case::no_scheme("not a url")]
#[case::empty("")]
fn a_base_url_that_does_not_parse_is_an_error(#[case] base: &str) {
    assert!(matches!(
        session_url(base, "s"),
        Err(FetchError::InvalidUrl(_))
    ));
}

fn failing_lookup(base_url: &str, key: &str, environment: &BTreeMap<String, String>) -> FetchError {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    let credentials = Credentials {
        base_url: base_url.to_owned(),
        api_key: SecretString::from(key.to_owned()),
    };
    HttpFetch {
        runtime: &runtime,
        environment,
    }
    .fetch(&credentials, "s")
    .unwrap_err()
}

#[test]
fn an_unusable_base_url_is_an_error_before_any_request() {
    let failed = failing_lookup("not a url", "key", &BTreeMap::new());

    assert!(matches!(failed, FetchError::InvalidUrl(_)));
}

#[test]
fn a_key_that_cannot_be_a_header_is_an_error_before_any_request() {
    let failed = failing_lookup("http://127.0.0.1:9", "bad\nkey", &BTreeMap::new());

    assert!(matches!(failed, FetchError::Http(_)));
}

#[test]
fn a_proxy_setting_that_cannot_be_read_is_an_error_before_any_request() {
    let environment =
        BTreeMap::from([("DISABLE_AIOHTTP_TRUST_ENV".to_owned(), "maybe".to_owned())]);

    let failed = failing_lookup("http://127.0.0.1:9", "key", &environment);

    assert!(matches!(failed, FetchError::Proxy(_)));
}
