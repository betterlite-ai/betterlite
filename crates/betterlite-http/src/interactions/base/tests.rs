#![cfg(test)]

use std::net::TcpListener;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_config::tls::Verification;
use betterlite_models::identity::provider_id::ProviderId;
use reqwest::blocking::ClientBuilder as BlockingClientBuilder;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use rstest::rstest;
use secrecy::SecretString;
use url::Url;

use super::clients::{HttpClients, async_client, client_policy};
use super::transport::response::{read_body, response_headers};
use super::transport::{query_pairs, refuse_unusable_proxy, request_headers, request_url};
use super::{
    CallFailure, ClientError, DEFAULT_PROVIDER, ErrorClasses, Params, PreparedRequest,
    RequestFailure, RequestHeaders, ResponseHeaders, api_base, handle_error,
};
use crate::proxy::base_builder;
use crate::proxy::limits::ReadError;

pub(crate) mod support;

#[derive(Debug, thiserror::Error)]
enum FixtureError {
    #[error("converted with status {status_code}: {message}")]
    Converted { status_code: u16, message: String },
}

#[derive(Default)]
struct Fixture {
    calls: Mutex<Vec<String>>,
}

impl Fixture {
    fn calls(&self) -> Vec<String> {
        match self.calls.lock() {
            Ok(calls) => calls.clone(),
            Err(_poisoned) => Vec::new(),
        }
    }
}

fn clients() -> HttpClients {
    HttpClients::new(
        support::policy(Verification::Enabled, None),
        support::proxy_policy(&[]),
    )
}

impl ErrorClasses for Fixture {
    type Error = FixtureError;

    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        headers: &ResponseHeaders,
    ) -> FixtureError {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(format!("{status_code}|{message}|{}", headers.len()));
        }
        FixtureError::Converted {
            status_code,
            message: message.to_owned(),
        }
    }
}

#[test]
fn handle_error_converts_a_status_failure_through_the_provider_class() {
    let fixture = Fixture::default();
    let mut headers = ResponseHeaders::new();
    headers.insert("retry-after".to_owned(), "2".to_owned());

    let failure = handle_error(
        &fixture,
        RequestFailure::Status {
            status_code: 429,
            headers,
            body: "slow down".to_owned(),
        },
    );

    match failure {
        CallFailure::Provider(FixtureError::Converted {
            status_code,
            message,
        }) => {
            assert_eq!(status_code, 429);
            assert_eq!(message, "slow down");
        }
        CallFailure::PassedThrough(other) => {
            panic!("expected the provider class, got {other:?}")
        }
    }
    assert_eq!(fixture.calls(), vec!["429|slow down|1".to_owned()]);
}

#[test]
fn handle_error_passes_a_transport_failure_through_unchanged() {
    let fixture = Fixture::default();

    let failure = handle_error(&fixture, RequestFailure::BodyTooLarge { limit: 4 });

    assert!(
        matches!(failure, CallFailure::PassedThrough(RequestFailure::BodyTooLarge { limit }) if limit == 4)
    );
    assert!(fixture.calls().is_empty());
}

#[test]
fn a_read_failure_keeps_its_limit() {
    let failure = RequestFailure::from(ReadError::TooLarge { limit: 4 });

    assert!(matches!(failure, RequestFailure::BodyTooLarge { limit } if limit == 4));
}

#[test]
fn sync_client_returns_the_supplied_client() {
    let serving = support::serve_once(
        support::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(200)),
    );
    let mut clients = clients();
    let supplied = BlockingClientBuilder::new()
        .timeout(Duration::from_millis(20))
        .build()
        .unwrap();

    let client = clients
        .sync_client(&support::params(r#"{"ssl_verify": "yes"}"#), Some(supplied))
        .unwrap();
    let outcome = client.get(format!("http://{}/x", serving.address)).send();

    assert!(matches!(&outcome, Err(source) if source.is_timeout()));
    assert!(clients.sync_clients.is_empty());
    serving.request.join().unwrap();
}

#[test]
fn sync_client_honors_ssl_verify_from_litellm_params() {
    let authority = support::tls_authority();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = support::spawn_tls_server(authority, listener);
    let mut clients = clients();
    let params = support::params(r#"{"ssl_verify": false}"#);

    let client = clients.sync_client(&params, None).unwrap();
    let outcome = client
        .get(format!("https://localhost:{}/x", address.port()))
        .send();

    assert!(outcome.is_ok(), "{outcome:?}");
    server.join().unwrap().unwrap();
}

#[test]
fn sync_client_verifies_a_certificate_without_an_override() {
    let authority = support::tls_authority();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = support::spawn_tls_server(authority, listener);
    let mut clients = clients();

    let client = clients.sync_client(&support::params("{}"), None).unwrap();
    let outcome = client
        .get(format!("https://localhost:{}/x", address.port()))
        .send();

    assert!(outcome.is_err(), "the fixture certificate is not trusted");
    server.join().unwrap().unwrap_err();
}

#[test]
fn sync_client_keeps_a_policy_that_skips_verification_without_an_override() {
    let authority = support::tls_authority();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = support::spawn_tls_server(authority, listener);
    let mut clients = HttpClients::new(
        support::policy(Verification::Disabled, None),
        support::proxy_policy(&[]),
    );

    let client = clients.sync_client(&support::params("{}"), None).unwrap();
    let outcome = client
        .get(format!("https://localhost:{}/x", address.port()))
        .send();

    assert!(outcome.is_ok(), "{outcome:?}");
    server.join().unwrap().unwrap();
}

#[tokio::test]
async fn async_client_honors_ssl_verify_from_litellm_params() {
    let authority = support::tls_authority();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = support::spawn_tls_server(authority, listener);
    let mut clients = clients();
    let params = support::params(r#"{"ssl_verify": false}"#);

    let client = async_client(&mut clients, &params, None).unwrap();
    let outcome = client
        .get(format!("https://localhost:{}/x", address.port()))
        .send()
        .await;

    assert!(outcome.is_ok(), "{outcome:?}");
    server.join().unwrap().unwrap();
}

#[test]
fn async_client_keys_its_cache_by_the_provider_from_litellm_params() {
    let mut clients = clients();
    let named_nothing = support::params("{}");
    let openai = support::params(r#"{"custom_llm_provider": "openai"}"#);

    async_client(&mut clients, &named_nothing, None).unwrap();
    async_client(&mut clients, &named_nothing, None).unwrap();
    assert_eq!(clients.async_clients.len(), 1);

    async_client(&mut clients, &openai, None).unwrap();
    let providers: Vec<ProviderId> = clients
        .async_clients
        .keys()
        .map(|key| key.provider)
        .collect();

    assert_eq!(clients.async_clients.len(), 2);
    assert!(providers.contains(&DEFAULT_PROVIDER));
    assert!(providers.contains(&ProviderId::Openai));
}

#[test]
fn async_client_falls_back_to_gemini_for_an_empty_provider() {
    let mut clients = clients();
    let empty = support::params(r#"{"custom_llm_provider": ""}"#);

    async_client(&mut clients, &empty, None).unwrap();

    let keys: Vec<_> = clients.async_clients.keys().collect();
    let expected = super::clients::ClientKey {
        provider: DEFAULT_PROVIDER,
        ssl_verify: None,
    };
    assert_eq!(keys, vec![&expected]);
}

#[test]
fn sync_client_builds_one_client_per_ssl_verify_setting() {
    let mut clients = clients();
    let unverified = support::params(r#"{"ssl_verify": false}"#);
    let inherited = support::params("{}");

    clients.sync_client(&unverified, None).unwrap();
    clients.sync_client(&inherited, None).unwrap();
    clients.sync_client(&unverified, None).unwrap();

    let keys: Vec<Option<bool>> = clients.sync_clients.keys().copied().collect();
    assert_eq!(keys, vec![None, Some(false)]);
}

#[test]
fn async_client_builds_one_client_per_ssl_verify_setting_under_one_provider() {
    let mut clients = clients();
    let unverified = support::params(r#"{"ssl_verify": false}"#);
    let inherited = support::params("{}");

    async_client(&mut clients, &unverified, None).unwrap();
    async_client(&mut clients, &inherited, None).unwrap();
    async_client(&mut clients, &unverified, None).unwrap();

    let keys: Vec<(ProviderId, Option<bool>)> = clients
        .async_clients
        .keys()
        .map(|key| (key.provider, key.ssl_verify))
        .collect();
    assert_eq!(
        keys,
        vec![(DEFAULT_PROVIDER, None), (DEFAULT_PROVIDER, Some(false))]
    );
}

#[rstest]
#[case::provider(r#"{"custom_llm_provider": null}"#)]
#[case::ssl_verify(r#"{"ssl_verify": null}"#)]
fn an_explicit_null_setting_counts_as_absent(#[case] text: &str) {
    let mut clients = clients();

    async_client(&mut clients, &support::params(text), None).unwrap();
    clients.sync_client(&support::params(text), None).unwrap();

    let keys: Vec<(ProviderId, Option<bool>)> = clients
        .async_clients
        .keys()
        .map(|key| (key.provider, key.ssl_verify))
        .collect();
    let sync_keys: Vec<Option<bool>> = clients.sync_clients.keys().copied().collect();
    assert_eq!(keys, vec![(DEFAULT_PROVIDER, None)]);
    assert_eq!(sync_keys, vec![None]);
}

#[test]
fn async_client_refuses_a_provider_it_does_not_know() {
    let mut clients = clients();
    let unknown = support::params(r#"{"custom_llm_provider": "nope"}"#);

    let outcome = async_client(&mut clients, &unknown, None);

    assert!(
        matches!(outcome, Err(ClientError::UnknownProvider { provider }) if provider == "nope")
    );
    assert!(clients.async_clients.is_empty());
}

#[test]
fn a_supplied_async_client_is_returned_before_the_provider_is_read() {
    let mut clients = clients();
    let unknown = support::params(r#"{"custom_llm_provider": "nope"}"#);
    let supplied = base_builder().build().unwrap();

    let outcome = async_client(&mut clients, &unknown, Some(supplied));

    outcome.unwrap();
    assert!(clients.async_clients.is_empty());
}

#[test]
fn sync_client_sends_the_request_through_the_configured_proxy() {
    let listener = support::listener();
    let proxy = format!("http://user:secret@{}", listener.local_addr().unwrap());
    let serving = support::serve_on(listener, support::reply("200 OK", &[], "{}"), None);
    let mut clients = HttpClients::new(
        support::policy(Verification::Enabled, None),
        support::proxy_policy(&[("HTTP_PROXY", proxy.as_str())]),
    );

    let client = clients.sync_client(&support::params("{}"), None).unwrap();
    let answer = client.get("http://127.0.0.1:1/x").send().unwrap();

    assert_eq!(answer.status().as_u16(), 200);
    let sent = serving.request.join().unwrap();
    assert!(
        sent.starts_with("GET http://127.0.0.1:1/x HTTP/1.1"),
        "{sent}"
    );
    assert!(
        sent.contains("proxy-authorization: Basic dXNlcjpzZWNyZXQ="),
        "{sent}"
    );
}

#[test]
fn a_proxy_for_the_scheme_beats_the_all_proxy() {
    let listener = support::listener();
    let proxy = format!("http://{}", listener.local_addr().unwrap());
    let serving = support::serve_on(listener, support::reply("200 OK", &[], "{}"), None);
    let mut clients = HttpClients::new(
        support::policy(Verification::Enabled, None),
        support::proxy_policy(&[
            ("ALL_PROXY", "http://127.0.0.1:1"),
            ("HTTP_PROXY", proxy.as_str()),
        ]),
    );

    let client = clients.sync_client(&support::params("{}"), None).unwrap();
    let answer = client.get("http://127.0.0.1:1/x").send().unwrap();

    assert_eq!(answer.status().as_u16(), 200);
    let sent = serving.request.join().unwrap();
    assert!(
        sent.starts_with("GET http://127.0.0.1:1/x HTTP/1.1"),
        "{sent}"
    );
}

#[tokio::test]
async fn async_client_sends_the_request_through_the_configured_proxy() {
    let listener = support::listener();
    let proxy = format!("http://{}", listener.local_addr().unwrap());
    let serving = support::serve_on(listener, support::reply("200 OK", &[], "{}"), None);
    let mut clients = HttpClients::new(
        support::policy(Verification::Enabled, None),
        support::proxy_policy(&[("HTTP_PROXY", proxy.as_str())]),
    );

    let client = async_client(&mut clients, &support::params("{}"), None).unwrap();
    let answer = client.get("http://127.0.0.1:1/x").send().await.unwrap();

    assert_eq!(answer.status().as_u16(), 200);
    let sent = serving.request.join().unwrap();
    assert!(
        sent.starts_with("GET http://127.0.0.1:1/x HTTP/1.1"),
        "{sent}"
    );
}

#[test]
fn a_host_the_policy_bypasses_skips_the_proxy() {
    let serving = support::serve_once(support::reply("200 OK", &[], "{}"), None);
    let mut clients = HttpClients::new(
        support::policy(Verification::Enabled, None),
        support::proxy_policy(&[
            ("HTTP_PROXY", "http://127.0.0.1:1"),
            ("NO_PROXY", "127.0.0.1"),
        ]),
    );

    let client = clients.sync_client(&support::params("{}"), None).unwrap();
    let answer = client
        .get(format!("http://{}/x", serving.address))
        .send()
        .unwrap();

    assert_eq!(answer.status().as_u16(), 200);
    let sent = serving.request.join().unwrap();
    assert_eq!(sent.lines().next(), Some("GET /x HTTP/1.1"));
}

#[rstest]
#[case::https_proxy("https://127.0.0.1:1", "UnsupportedProxyScheme { scheme: \"https\" }")]
#[case::unparsable_proxy("http://", "InvalidProxyUrl { name: \"http\" }")]
fn a_proxy_setting_betterlite_cannot_use_is_refused(#[case] proxy: &str, #[case] expected: &str) {
    let policy = support::proxy_policy(&[("HTTP_PROXY", proxy)]);
    let url = Url::parse("http://provider.example/x").unwrap();

    let failure = refuse_unusable_proxy(&policy, &url).unwrap_err();

    assert_eq!(format!("{failure:?}"), expected);
}

#[rstest]
#[case::forward_route(&[("HTTP_PROXY", "http://127.0.0.1:1")])]
#[case::no_route(&[])]
#[case::other_scheme(&[("HTTPS_PROXY", "https://127.0.0.1:1")])]
fn a_proxy_setting_betterlite_can_use_passes(#[case] variables: &[(&str, &str)]) {
    let policy = support::proxy_policy(variables);
    let url = Url::parse("http://provider.example/x").unwrap();

    refuse_unusable_proxy(&policy, &url).unwrap();
}

#[rstest]
#[case::absent("{}", "")]
#[case::null(r#"{"api_base": null}"#, "")]
#[case::text(r#"{"api_base": "http://host/v1"}"#, "http://host/v1")]
fn api_base_reads_the_base_or_yields_an_empty_string(#[case] text: &str, #[case] expected: &str) {
    let params = support::params(text);

    assert_eq!(api_base(&params).unwrap(), expected);
}

#[test]
fn api_base_refuses_a_value_that_is_not_a_string() {
    let params = support::params(r#"{"api_base": 3}"#);

    let failure = api_base(&params).unwrap_err();

    assert!(matches!(
        failure,
        ClientError::FieldType {
            field: "api_base",
            expected: "a string"
        }
    ));
}

#[rstest]
#[case::ssl_verify(r#"{"ssl_verify": "yes"}"#, "ssl_verify")]
#[case::provider(r#"{"custom_llm_provider": 3}"#, "custom_llm_provider")]
fn a_param_of_the_wrong_type_names_the_field(#[case] text: &str, #[case] field: &str) {
    let mut clients = clients();
    let params = support::params(text);

    let outcome = async_client(&mut clients, &params, None);

    assert!(matches!(outcome, Err(ClientError::FieldType { field: named, .. }) if named == field));
}

#[rstest]
#[case::inherited(Verification::Enabled, None, Verification::Enabled)]
#[case::disabled_by_params(Verification::Enabled, Some(false), Verification::Disabled)]
#[case::enabled_by_params(Verification::Disabled, Some(true), Verification::Enabled)]
#[case::inherited_disabled(Verification::Disabled, None, Verification::Disabled)]
#[case::ca_file_kept(Verification::CaFile("ca.pem".to_owned()), Some(true), Verification::CaFile("ca.pem".to_owned()))]
#[case::ca_file_inherited(Verification::CaFile("ca.pem".to_owned()), None, Verification::CaFile("ca.pem".to_owned()))]
fn the_ssl_verify_setting_overrides_the_policy(
    #[case] base: Verification,
    #[case] ssl_verify: Option<bool>,
    #[case] expected: Verification,
) {
    let ca_file = match &base {
        Verification::CaFile(path) => Some(path.clone()),
        Verification::Enabled | Verification::Disabled => None,
    };
    let policy = support::policy(base, ca_file.as_deref());

    assert_eq!(client_policy(&policy, ssl_verify).verification, expected);
}

#[rstest]
#[case::text(r#"{"alt": "sse"}"#, vec![("alt", "sse")])]
#[case::number(r#"{"count": 3}"#, vec![("count", "3")])]
#[case::boolean(r#"{"flag": true}"#, vec![("flag", "true")])]
#[case::repeated(r#"{"tag": ["a", "b"]}"#, vec![("tag", "a"), ("tag", "b")])]
#[case::null_empty(r#"{"gone": null}"#, vec![("gone", "")])]
#[case::null_in_a_list(r#"{"tag": ["a", null]}"#, vec![("tag", "a"), ("tag", "")])]
fn query_pairs_carry_the_transformed_params(
    #[case] text: &str,
    #[case] expected: Vec<(&str, &str)>,
) {
    let pairs = query_pairs(&support::params(text)).unwrap();
    let owned: Vec<(String, String)> = expected
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();

    assert_eq!(pairs, owned);
}

#[rstest]
#[case::object(r#"{"nested": {"a": 1}}"#)]
#[case::nested_list(r#"{"nested": [["a"]]}"#)]
fn a_query_value_that_is_not_a_scalar_is_refused(#[case] text: &str) {
    let failure = query_pairs(&support::params(text)).unwrap_err();

    assert!(matches!(failure, RequestFailure::QueryValue { field } if field == "nested"));
}

#[rstest]
#[case::joined("http://host/x", r#"{"b": "2"}"#, "http://host/x?b=2")]
#[case::url_first("http://host/x?a=1", r#"{"b": "2"}"#, "http://host/x?a=1&b=2")]
#[case::url_key_wins("http://host/x?a=1", r#"{"a": "2"}"#, "http://host/x?a=1")]
fn request_url_joins_the_transformed_params(
    #[case] url: &str,
    #[case] text: &str,
    #[case] expected: &str,
) {
    let prepared = PreparedRequest {
        url: url.to_owned(),
        params: support::params(text),
    };

    assert_eq!(request_url(&prepared).unwrap().as_str(), expected);
}

#[test]
fn request_url_refuses_a_url_it_cannot_parse() {
    let prepared = PreparedRequest {
        url: "not a url".to_owned(),
        params: Params::new(),
    };

    let failure = request_url(&prepared).unwrap_err();

    assert!(matches!(failure, RequestFailure::Url(_)));
}

#[test]
fn request_headers_carry_the_secret_values() {
    let mut headers = RequestHeaders::new();
    headers.insert("x-goog-api-key".to_owned(), SecretString::from("key-1"));

    let map = request_headers(&headers).unwrap();

    assert_eq!(
        map.get("x-goog-api-key").unwrap().to_str().unwrap(),
        "key-1"
    );
}

#[rstest]
#[case::name("bad header", "value")]
#[case::value("x-goog-api-key", "line\nbreak")]
fn an_invalid_request_header_is_refused(#[case] name: &str, #[case] value: &str) {
    let mut headers = RequestHeaders::new();
    headers.insert(name.to_owned(), SecretString::from(value));

    request_headers(&headers).unwrap_err();
}

#[test]
fn response_headers_carry_the_values() {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-request-id"),
        HeaderValue::from_static("r1"),
    );

    let map = response_headers(&headers);

    assert_eq!(map.get("x-request-id").map(String::as_str), Some("r1"));
}

#[test]
fn repeated_response_headers_join_with_a_comma() {
    let mut headers = HeaderMap::new();
    headers.append(
        HeaderName::from_static("x-many"),
        HeaderValue::from_static("a"),
    );
    headers.append(
        HeaderName::from_static("x-many"),
        HeaderValue::from_static("b"),
    );

    let map = response_headers(&headers);

    assert_eq!(map.get("x-many").map(String::as_str), Some("a, b"));
}

#[test]
fn a_response_header_that_is_not_utf8_decodes_as_iso_8859_1() {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-raw"),
        HeaderValue::from_bytes(b"\xff").unwrap(),
    );

    let map = response_headers(&headers);

    assert_eq!(map.get("x-raw").map(String::as_str), Some("\u{ff}"));
}

#[rstest]
#[case::under(b"abc".as_slice(), 4)]
#[case::exactly(b"abcd".as_slice(), 4)]
fn reading_a_body_within_the_limit_returns_it(#[case] body: &[u8], #[case] limit: usize) {
    let read = read_body(body, limit).unwrap();

    assert_eq!(read, body);
}

#[test]
fn reading_a_body_over_the_limit_is_refused() {
    let failure = read_body(b"abcde".as_slice(), 4).unwrap_err();

    assert!(matches!(failure, RequestFailure::BodyTooLarge { limit } if limit == 4));
}
