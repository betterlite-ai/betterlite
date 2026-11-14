#![cfg(test)]

use std::net::{SocketAddr, TcpListener};
use std::str::{Utf8Error, from_utf8};
use std::sync::Mutex;
use std::time::Duration;

use betterlite_config::tls::Verification;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::identity::provider_id::ProviderId;
use reqwest::blocking::ClientBuilder as BlockingClientBuilder;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::asynchronous::async_get_interaction;
use super::synchronous::get_interaction;
use super::{
    EnvironmentRequest, GetInteractionConfig, GetInteractionError, GetInteractionRequest,
    InteractionIdRequest,
};
use crate::interactions::base::clients::HttpClients;
use crate::interactions::base::tests::support;
use crate::interactions::base::{
    ErrorClasses, Logging, Params, PreparedRequest, RequestFailure, RequestHeaders,
    ResponseHeaders, UpstreamResponse,
};
use crate::proxy::base_builder;

#[derive(Debug, thiserror::Error)]
enum FixtureError {
    #[error("refused with status {0}")]
    Refused(u16),
    #[error("the body is not text")]
    Body(#[source] Utf8Error),
}

#[derive(Default)]
struct Fixture {
    calls: Mutex<Vec<String>>,
    params: Params,
    headers: RequestHeaders,
    refuse_status: Option<u16>,
}

impl Fixture {
    fn record(&self, call: String) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(call);
        }
    }

    fn calls(&self) -> Vec<String> {
        match self.calls.lock() {
            Ok(calls) => calls.clone(),
            Err(_poisoned) => Vec::new(),
        }
    }
}

impl ErrorClasses for Fixture {
    type Error = FixtureError;

    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        headers: &ResponseHeaders,
    ) -> FixtureError {
        self.record(format!("error|{status_code}|{message}|{}", headers.len()));
        FixtureError::Refused(status_code)
    }
}

impl GetInteractionConfig for Fixture {
    type Output = String;

    fn validate_environment(
        &self,
        request: &EnvironmentRequest<'_>,
    ) -> Result<RequestHeaders, FixtureError> {
        self.record(format!(
            "environment|{}|{}|{}",
            request.model,
            request.headers.len(),
            request.litellm_params.map_or(0, Params::len)
        ));
        Ok(self.headers.clone())
    }

    fn transform_get_request(
        &self,
        request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, FixtureError> {
        self.record(format!(
            "request|{}|{}|{}",
            request.interaction_id,
            request.headers.len(),
            request.litellm_params.len()
        ));
        Ok(PreparedRequest {
            url: format!(
                "{}/v1beta/interactions/{}",
                request.api_base, request.interaction_id
            ),
            params: self.params.clone(),
        })
    }

    fn transform_get_response(
        &self,
        response: &UpstreamResponse,
        logging: &mut Logging,
    ) -> Result<String, FixtureError> {
        let input = logging
            .pre_call
            .as_ref()
            .map_or_else(String::new, |call| call.input.clone());
        let request_id = response
            .headers
            .get("x-request-id")
            .map_or("-", String::as_str);
        self.record(format!(
            "response|{}|{input}|{request_id}",
            response.status_code
        ));
        logging
            .model_call_details
            .insert("seen".to_owned(), JsonValue::Bool(true));
        if self.refuse_status == Some(response.status_code) {
            return Err(FixtureError::Refused(response.status_code));
        }
        from_utf8(&response.body)
            .map(str::to_owned)
            .map_err(FixtureError::Body)
    }
}

struct Call {
    params: Params,
    provider: ProviderId,
    extra_headers: RequestHeaders,
    timeout: Option<Duration>,
}

impl Call {
    fn new(address: SocketAddr) -> Self {
        let mut params = Params::new();
        params.insert(
            "api_base".to_owned(),
            JsonValue::Text(format!("http://{address}")),
        );
        Self {
            params,
            provider: ProviderId::Gemini,
            extra_headers: RequestHeaders::new(),
            timeout: None,
        }
    }

    fn request(&self) -> GetInteractionRequest<'_> {
        GetInteractionRequest {
            interaction_id: "int_1",
            litellm_params: &self.params,
            provider: self.provider,
            extra_headers: self.extra_headers.clone(),
            timeout: self.timeout,
        }
    }
}

fn clients() -> HttpClients {
    HttpClients::new(
        support::policy(Verification::Enabled, None),
        support::proxy_policy(&[]),
    )
}

fn dead_address() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    address
}

fn caller_headers(entries: &[(&str, &str)]) -> RequestHeaders {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), SecretString::from(*value)))
        .collect()
}

#[test]
fn get_interaction_retrieves_the_interaction_through_the_config() {
    let serving = support::serve_once(
        support::reply("200 OK", &[("x-request-id", "r1")], "{\"id\":\"int_1\"}"),
        None,
    );
    let mut fixture = Fixture::default();
    fixture
        .headers
        .insert("x-goog-api-key".to_owned(), SecretString::from("key-1"));
    fixture
        .headers
        .insert("x-validated".to_owned(), SecretString::from("v1"));
    let mut clients = clients();
    let mut logging = Logging::default();
    let mut call = Call::new(serving.address);
    call.params.insert(
        "custom_llm_provider".to_owned(),
        JsonValue::Text("gemini".to_owned()),
    );
    call.extra_headers = caller_headers(&[("x-caller", "c1")]);

    let output =
        get_interaction(&fixture, &mut clients, &mut logging, None, &call.request()).unwrap();

    assert_eq!(output, "{\"id\":\"int_1\"}");
    let sent = serving.request.join().unwrap();
    assert!(
        sent.starts_with("GET /v1beta/interactions/int_1 HTTP/1.1"),
        "{sent}"
    );
    assert!(sent.contains("x-goog-api-key: key-1\r\n"), "{sent}");
    assert_eq!(
        fixture.calls(),
        vec![
            "environment||1|2".to_owned(),
            "request|int_1|2|2".to_owned(),
            "response|200|int_1|r1".to_owned(),
        ]
    );
    assert_eq!(
        logging.model_call_details.get("seen"),
        Some(&JsonValue::Bool(true))
    );
}

#[test]
fn get_interaction_records_the_pre_call_with_the_interaction_id() {
    let serving = support::serve_once(support::reply("200 OK", &[], "{}"), None);
    let mut fixture = Fixture::default();
    fixture
        .headers
        .insert("x-goog-api-key".to_owned(), SecretString::from("key-1"));
    let mut clients = clients();
    let mut logging = Logging::default();

    get_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &Call::new(serving.address).request(),
    )
    .unwrap();

    let recorded = logging.pre_call.as_ref().unwrap();
    assert_eq!(recorded.input, "int_1");
    assert_eq!(recorded.api_key.expose_secret(), "");
    assert_eq!(
        recorded.api_base,
        format!("http://{}/v1beta/interactions/int_1", serving.address)
    );
    assert_eq!(recorded.headers["x-goog-api-key"].expose_secret(), "key-1");
    let sent = serving.request.join().unwrap();
    assert!(sent.contains("x-goog-api-key: key-1\r\n"), "{sent}");
}

#[test]
fn get_interaction_takes_the_api_base_from_litellm_params() {
    let serving = support::serve_once(support::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::default();
    let mut clients = clients();
    let mut logging = Logging::default();
    let mut call = Call::new(serving.address);
    call.params.insert(
        "api_base".to_owned(),
        JsonValue::Text(format!("http://{}/base", serving.address)),
    );

    get_interaction(&fixture, &mut clients, &mut logging, None, &call.request()).unwrap();

    let sent = serving.request.join().unwrap();
    assert!(
        sent.starts_with("GET /base/v1beta/interactions/int_1 HTTP/1.1"),
        "{sent}"
    );
    let recorded = logging.pre_call.as_ref().unwrap();
    assert_eq!(
        recorded.api_base,
        format!("http://{}/base/v1beta/interactions/int_1", serving.address)
    );
}

#[test]
fn get_interaction_sends_no_deadline_of_its_own() {
    let serving = support::serve_once(
        support::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(100)),
    );
    let fixture = Fixture::default();
    let mut clients = clients();
    let mut logging = Logging::default();
    let mut call = Call::new(serving.address);
    call.timeout = Some(Duration::from_millis(1));

    let output =
        get_interaction(&fixture, &mut clients, &mut logging, None, &call.request()).unwrap();

    assert_eq!(output, "{}");
    serving.request.join().unwrap();
}

#[test]
fn get_interaction_sends_the_transformed_query_params() {
    let serving = support::serve_once(support::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture {
        params: support::params(r#"{"alt": "sse"}"#),
        ..Fixture::default()
    };
    let mut clients = clients();
    let mut logging = Logging::default();

    get_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &Call::new(serving.address).request(),
    )
    .unwrap();

    let sent = serving.request.join().unwrap();
    assert!(
        sent.starts_with("GET /v1beta/interactions/int_1?alt=sse HTTP/1.1"),
        "{sent}"
    );
}

#[rstest]
#[case::absent(&[], "environment||0|1")]
#[case::given(&[("x-caller", "c1")], "environment||1|1")]
fn get_interaction_sends_the_caller_headers_to_validate_environment(
    #[case] extra: &[(&str, &str)],
    #[case] expected: &str,
) {
    let serving = support::serve_once(support::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::default();
    let mut clients = clients();
    let mut logging = Logging::default();
    let mut call = Call::new(serving.address);
    call.extra_headers = caller_headers(extra);

    get_interaction(&fixture, &mut clients, &mut logging, None, &call.request()).unwrap();

    let calls = fixture.calls();
    assert_eq!(calls.first().map(String::as_str), Some(expected));
    serving.request.join().unwrap();
}

#[test]
fn get_interaction_passes_a_failing_status_to_the_response_transform() {
    let serving = support::serve_once(support::reply("404 Not Found", &[], "missing"), None);
    let fixture = Fixture {
        refuse_status: Some(404),
        ..Fixture::default()
    };
    let mut clients = clients();
    let mut logging = Logging::default();

    let error = get_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &Call::new(serving.address).request(),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        GetInteractionError::Provider(FixtureError::Refused(404))
    ));
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| !call.starts_with("error|"))
    );
    serving.request.join().unwrap();
}

#[test]
fn get_interaction_does_not_follow_a_redirect() {
    let serving = support::serve_once(
        support::reply(
            "302 Found",
            &[("location", "http://127.0.0.1:1/elsewhere")],
            "",
        ),
        None,
    );
    let fixture = Fixture::default();
    let mut clients = clients();
    let mut logging = Logging::default();

    let output = get_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &Call::new(serving.address).request(),
    )
    .unwrap();

    assert_eq!(output, "");
    let calls = fixture.calls();
    assert!(
        calls.contains(&"response|302|int_1|-".to_owned()),
        "{calls:?}"
    );
    serving.request.join().unwrap();
}

#[test]
fn get_interaction_passes_a_transport_failure_through_the_error_handler() {
    let fixture = Fixture::default();
    let mut clients = clients();
    let mut logging = Logging::default();

    let error = get_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &Call::new(dead_address()).request(),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        GetInteractionError::Transport(RequestFailure::Transport(_))
    ));
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| !call.starts_with("error|"))
    );
}

#[test]
fn get_interaction_passes_an_invalid_url_through_the_error_handler() {
    let fixture = Fixture::default();
    let mut clients = clients();
    let mut logging = Logging::default();
    let mut call = Call::new(dead_address());
    call.params.insert(
        "api_base".to_owned(),
        JsonValue::Text("not a url".to_owned()),
    );

    let error =
        get_interaction(&fixture, &mut clients, &mut logging, None, &call.request()).unwrap_err();

    assert!(matches!(
        error,
        GetInteractionError::Transport(RequestFailure::Url(_))
    ));
}

#[test]
fn get_interaction_uses_a_supplied_client() {
    let serving = support::serve_once(
        support::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(200)),
    );
    let fixture = Fixture::default();
    let mut clients = clients();
    let mut logging = Logging::default();
    let supplied = BlockingClientBuilder::new()
        .timeout(Duration::from_millis(20))
        .build()
        .unwrap();

    let error = get_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        Some(supplied),
        &Call::new(serving.address).request(),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        GetInteractionError::Transport(RequestFailure::Transport(source)) if source.is_timeout()
    ));
    assert!(clients.sync_clients.is_empty());
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_get_interaction_retrieves_the_interaction_through_the_config() {
    let serving = support::serve_once(support::reply("200 OK", &[], "{}"), None);
    let mut fixture = Fixture::default();
    fixture
        .headers
        .insert("x-goog-api-key".to_owned(), SecretString::from("key-1"));
    fixture.params = support::params(r#"{"alt": "sse"}"#);
    let mut clients = clients();
    let mut logging = Logging::default();

    let output = async_get_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &Call::new(serving.address).request(),
    )
    .await
    .unwrap();

    assert_eq!(output, "{}");
    let sent = serving.request.join().unwrap();
    assert!(
        sent.starts_with("GET /v1beta/interactions/int_1?alt=sse HTTP/1.1"),
        "{sent}"
    );
    assert!(sent.contains("x-goog-api-key: key-1\r\n"), "{sent}");
    let recorded = logging.pre_call.as_ref().unwrap();
    assert_eq!(recorded.input, "int_1");
}

#[tokio::test]
async fn async_get_interaction_uses_a_supplied_client() {
    let serving = support::serve_once(
        support::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(200)),
    );
    let fixture = Fixture::default();
    let mut clients = clients();
    let mut logging = Logging::default();
    let supplied = base_builder()
        .timeout(Duration::from_millis(20))
        .build()
        .unwrap();

    let error = async_get_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        Some(supplied),
        &Call::new(serving.address).request(),
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        GetInteractionError::Transport(RequestFailure::Transport(source)) if source.is_timeout()
    ));
    assert!(clients.async_clients.is_empty());
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_get_interaction_builds_for_the_provider_its_caller_names() {
    let serving = support::serve_once(support::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::default();
    let mut clients = clients();
    let mut logging = Logging::default();
    let mut call = Call::new(serving.address);
    call.provider = ProviderId::Openai;

    let output = async_get_interaction(&fixture, &mut clients, &mut logging, None, &call.request())
        .await
        .unwrap();

    assert_eq!(output, "{}");
    let providers: Vec<ProviderId> = clients
        .async_clients
        .keys()
        .map(|key| key.provider)
        .collect();
    assert!(providers.contains(&ProviderId::Openai));
    assert!(!providers.contains(&ProviderId::Gemini));
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_get_interaction_passes_a_failing_status_to_the_response_transform() {
    let serving = support::serve_once(
        support::reply("500 Internal Server Error", &[], "boom"),
        None,
    );
    let fixture = Fixture {
        refuse_status: Some(500),
        ..Fixture::default()
    };
    let mut clients = clients();
    let mut logging = Logging::default();

    let error = async_get_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &Call::new(serving.address).request(),
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        GetInteractionError::Provider(FixtureError::Refused(500))
    ));
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| !call.starts_with("error|"))
    );
    serving.request.join().unwrap();
}

#[rstest]
#[case::unsupported_scheme("https://127.0.0.1:1", "UnsupportedProxyScheme { scheme: \"https\" }")]
#[case::invalid_url("http://", "InvalidProxyUrl { name: \"http\" }")]
fn get_interaction_refuses_a_proxy_it_cannot_use_instead_of_connecting_directly(
    #[case] proxy: &str,
    #[case] expected: &str,
) {
    let fixture = Fixture::default();
    let mut clients = HttpClients::new(
        support::policy(Verification::Enabled, None),
        support::proxy_policy(&[("HTTP_PROXY", proxy)]),
    );
    let mut logging = Logging::default();
    let call = Call::new(dead_address());

    let error =
        get_interaction(&fixture, &mut clients, &mut logging, None, &call.request()).unwrap_err();

    assert!(
        matches!(&error, GetInteractionError::Transport(failure) if format!("{failure:?}") == expected),
        "{error:?}"
    );
}

#[test]
fn get_interaction_skips_the_proxy_check_for_a_supplied_client() {
    let serving = support::serve_once(support::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::default();
    let mut clients = HttpClients::new(
        support::policy(Verification::Enabled, None),
        support::proxy_policy(&[("HTTP_PROXY", "https://127.0.0.1:1")]),
    );
    let mut logging = Logging::default();
    let supplied = BlockingClientBuilder::new().build().unwrap();

    let output = get_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        Some(supplied),
        &Call::new(serving.address).request(),
    )
    .unwrap();

    assert_eq!(output, "{}");
    serving.request.join().unwrap();
}

#[rstest]
#[case::unsupported_scheme("https://127.0.0.1:1", "UnsupportedProxyScheme { scheme: \"https\" }")]
#[case::invalid_url("http://", "InvalidProxyUrl { name: \"http\" }")]
#[tokio::test]
async fn async_get_interaction_refuses_a_proxy_it_cannot_use_instead_of_connecting_directly(
    #[case] proxy: &str,
    #[case] expected: &str,
) {
    let fixture = Fixture::default();
    let mut clients = HttpClients::new(
        support::policy(Verification::Enabled, None),
        support::proxy_policy(&[("HTTP_PROXY", proxy)]),
    );
    let mut logging = Logging::default();
    let call = Call::new(dead_address());

    let error = async_get_interaction(&fixture, &mut clients, &mut logging, None, &call.request())
        .await
        .unwrap_err();

    assert!(
        matches!(&error, GetInteractionError::Transport(failure) if format!("{failure:?}") == expected),
        "{error:?}"
    );
}

#[tokio::test]
async fn async_get_interaction_skips_the_proxy_check_for_a_supplied_client() {
    let serving = support::serve_once(support::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::default();
    let mut clients = HttpClients::new(
        support::policy(Verification::Enabled, None),
        support::proxy_policy(&[("HTTP_PROXY", "https://127.0.0.1:1")]),
    );
    let mut logging = Logging::default();
    let supplied = base_builder().build().unwrap();

    let output = async_get_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        Some(supplied),
        &Call::new(serving.address).request(),
    )
    .await
    .unwrap();

    assert_eq!(output, "{}");
    serving.request.join().unwrap();
}
