#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::proxy::{ProxyPolicy, ProxyRoute};
use betterlite_config::types::ProxyTrust;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use http::HeaderMap;
use http::header::ACCEPT;
use rstest::rstest;
use secrecy::SecretString;
use tokio::runtime::Builder;
use url::Url;

use super::error::FetchError;
use super::{
    ACCEPT_JSON, Fetched, HttpFetch, REQUEST_TIMEOUT, SessionFetch as _, classify, fetched_answer,
    proxy_environment, session_url,
};
use crate::statusline::credentials::Credentials;
use crate::statusline::session::Session;

const VALID_BODY: &str = r#"{"router_name":"auto","last_model":"gpt-5","spend":1.5}"#;

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn transport_error() -> HttpError {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    let client = ProxiedClient::outbound(&Arc::new(policy)).unwrap();
    HttpError::Request(client.client.get("not a url").build().unwrap_err())
}

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

#[test]
fn a_received_answer_is_classified_by_its_status_and_body() {
    let answer = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: VALID_BODY.as_bytes().to_vec(),
    };

    let fetched = fetched_answer(Ok(answer));

    assert_eq!(fetched, classify(200, VALID_BODY.as_bytes()));
    assert!(fetched.definitive);
}

#[rstest]
#[case::transport_error(transport_error())]
#[case::timeout(HttpError::Timeout)]
#[case::oversized_body(HttpError::TooLarge { limit: 1_048_576 })]
fn a_failed_transport_is_not_an_answer(#[case] failure: HttpError) {
    let fetched = fetched_answer(Err(failure));

    assert_eq!(fetched, Fetched::default());
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
fn the_request_sends_the_bearer_key_accept_json_and_a_three_second_timeout() {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    let client = ProxiedClient::outbound(&Arc::new(policy)).unwrap();
    let url = session_url("http://proxy", "abc").unwrap();

    let request = session_request!(client, url, "sk-key").unwrap();

    assert_eq!(
        request.url().as_str(),
        "http://proxy/auto_router/session?session_id=abc"
    );
    assert_eq!(request.timeout(), Some(&REQUEST_TIMEOUT));
    assert_eq!(request.headers()["authorization"], "Bearer sk-key");
    assert_eq!(request.headers()["accept"], "application/json");
}

#[rstest]
#[case::a_trust_env_flag_does_not_disable_the_proxy(
    &[
        ("HTTPS_PROXY", "http://egress.test:3128"),
        ("DISABLE_AIOHTTP_TRUST_ENV", "true"),
    ],
    true
)]
#[case::a_flag_that_cannot_be_read_is_not_read(
    &[
        ("HTTPS_PROXY", "http://egress.test:3128"),
        ("DISABLE_AIOHTTP_TRUST_ENV", "maybe"),
        ("AIOHTTP_TRUST_ENV", "maybe"),
    ],
    true
)]
#[case::no_proxy_still_bypasses(
    &[
        ("HTTPS_PROXY", "http://egress.test:3128"),
        ("NO_PROXY", "gateway.test"),
    ],
    false
)]
#[case::only_proxy_variables_are_passed_on(
    &[
        ("HTTPS_PROXY", "http://egress.test:3128"),
        ("LITELLM_TRUST_ENV", "maybe"),
    ],
    true
)]
fn the_proxy_comes_from_the_urllib_variables_alone(
    #[case] pairs: &[(&str, &str)],
    #[case] routed: bool,
) {
    let environment = environment(pairs);
    let policy =
        ProxyPolicy::resolve(&ProxyTrust::default(), &proxy_environment(&environment)).unwrap();
    let url = Url::parse("https://gateway.test/auto_router/session").unwrap();

    assert_eq!(
        matches!(policy.route(&url), Some(ProxyRoute::Forward(_))),
        routed
    );
}
