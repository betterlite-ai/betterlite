#![cfg(test)]

use std::net::{SocketAddr, TcpListener};
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
    Body(#[source] std::str::Utf8Error),
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
            "request|{}|{}",
            request.interaction_id,
            request.headers.len()
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
        std::str::from_utf8(&response.body)
            .map(str::to_owned)
            .map_err(FixtureError::Body)
    }
}

struct Call {
    api_base: String,
    params: Params,
    extra_headers: Option<RequestHeaders>,
    timeout: Option<Duration>,
}

impl Call {
    fn new(address: SocketAddr) -> Self {
        Self {
            api_base: format!("http://{address}"),
            params: Params::new(),
            extra_headers: None,
            timeout: None,
        }
    }

    fn request(&self) -> GetInteractionRequest<'_> {
        GetInteractionRequest {
            interaction_id: "int_1",
            api_base: &self.api_base,
            litellm_params: &self.params,
            provider: ProviderId::Gemini,
            extra_headers: self.extra_headers.clone(),
            timeout: self.timeout,
        }
    }
}

fn dead_address() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    address
}

fn caller_headers(entries: Option<Vec<(&str, &str)>>) -> Option<RequestHeaders> {
    entries.map(|pairs| {
        pairs
            .into_iter()
            .map(|(name, value)| (name.to_owned(), SecretString::from(value)))
            .collect()
    })
}

#[test]
fn get_interaction_retrieves_the_interaction_through_the_config() {
    let serving = support::serve_once(
        support::reply("200 OK", &[("x-request-id", "r1")], "{\"id\":\"int_1\"}"),
        None,
    );
    let fixture = Fixture::default();
    let mut clients = HttpClients::new(support::policy(Verification::Enabled, None));
    let mut logging = Logging::default();

    let output = get_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        Call::new(serving.address).request(),
    )
    .unwrap();

    assert_eq!(output, "{\"id\":\"int_1\"}");
    let sent = serving.request.join().unwrap();
    assert!(
        sent.starts_with("GET /v1beta/interactions/int_1 HTTP/1.1"),
        "{sent}"
    );
    assert_eq!(
        fixture.calls(),
        vec![
            "environment||0|0".to_owned(),
            "request|int_1|0".to_owned(),
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
    let mut clients = HttpClients::new(support::policy(Verification::Enabled, None));
    let mut logging = Logging::default();

    get_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        Call::new(serving.address).request(),
    )
    .unwrap();

    let recorded = logging.pre_call.as_ref().unwrap();
    assert_eq!(recorded.input, "int_1");
    assert_eq!(recorded.api_key.expose_secret(), "");
    assert_eq!(
        recorded.api_base,
        format!("http://{}/v1beta/interactions/int_1", serving.address)
    );
    assert_eq!(
        recorded
            .headers
            .get("x-goog-api-key")
            .unwrap()
            .expose_secret(),
        "key-1"
    );
    let sent = serving.request.join().unwrap();
    assert!(sent.contains("x-goog-api-key: key-1\r\n"), "{sent}");
}

#[test]
fn get_interaction_sends_no_deadline_of_its_own() {
    let serving = support::serve_once(
        support::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(100)),
    );
    let fixture = Fixture::default();
    let mut clients = HttpClients::new(support::policy(Verification::Enabled, None));
    let mut logging = Logging::default();
    let mut call = Call::new(serving.address);
    call.timeout = Some(Duration::from_millis(1));

    let output =
        get_interaction(&fixture, &mut clients, &mut logging, None, call.request()).unwrap();

    assert_eq!(output, "{}");
    serving.request.join().unwrap();
}

#[test]
fn get_interaction_sends_the_transformed_query_params() {
    let serving = support::serve_once(support::reply("200 OK", &[], "{}"), None);
    let mut fixture = Fixture::default();
    fixture.params = support::params(r#"{"alt": "sse"}"#);
    let mut clients = HttpClients::new(support::policy(Verification::Enabled, None));
    let mut logging = Logging::default();

    get_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        Call::new(serving.address).request(),
    )
    .unwrap();

    let sent = serving.request.join().unwrap();
    assert!(
        sent.starts_with("GET /v1beta/interactions/int_1?alt=sse HTTP/1.1"),
        "{sent}"
    );
}

#[rstest]
#[case::absent(None, "environment||0|0")]
#[case::given(Some(vec![("x-caller", "c1")]), "environment||1|0")]
fn get_interaction_sends_the_caller_headers_to_validate_environment(
    #[case] extra: Option<Vec<(&str, &str)>>,
    #[case] expected: &str,
) {
    let serving = support::serve_once(support::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::default();
    let mut clients = HttpClients::new(support::policy(Verification::Enabled, None));
    let mut logging = Logging::default();
    let mut call = Call::new(serving.address);
    call.extra_headers = caller_headers(extra);

    get_interaction(&fixture, &mut clients, &mut logging, None, call.request()).unwrap();

    let calls = fixture.calls();
    assert_eq!(calls.first().map(String::as_str), Some(expected));
    serving.request.join().unwrap();
}

#[test]
fn get_interaction_passes_a_failing_status_to_the_response_transform() {
    let serving = support::serve_once(support::reply("404 Not Found", &[], "missing"), None);
    let mut fixture = Fixture::default();
    fixture.refuse_status = Some(404);
    let mut clients = HttpClients::new(support::policy(Verification::Enabled, None));
    let mut logging = Logging::default();

    let error = get_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        Call::new(serving.address).request(),
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
fn get_interaction_passes_a_transport_failure_through_the_error_handler() {
    let fixture = Fixture::default();
    let mut clients = HttpClients::new(support::policy(Verification::Enabled, None));
    let mut logging = Logging::default();

    let error = get_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        Call::new(dead_address()).request(),
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
    let mut clients = HttpClients::new(support::policy(Verification::Enabled, None));
    let mut logging = Logging::default();
    let mut call = Call::new(dead_address());
    call.api_base = "not a url".to_owned();

    let error =
        get_interaction(&fixture, &mut clients, &mut logging, None, call.request()).unwrap_err();

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
    let mut clients = HttpClients::new(support::policy(Verification::Enabled, None));
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
        Call::new(serving.address).request(),
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
    let fixture = Fixture::default();
    let mut clients = HttpClients::new(support::policy(Verification::Enabled, None));
    let mut logging = Logging::default();

    let output = async_get_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        Call::new(serving.address).request(),
    )
    .await
    .unwrap();

    assert_eq!(output, "{}");
    let sent = serving.request.join().unwrap();
    assert!(
        sent.starts_with("GET /v1beta/interactions/int_1 HTTP/1.1"),
        "{sent}"
    );
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
    let mut clients = HttpClients::new(support::policy(Verification::Enabled, None));
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
        Call::new(serving.address).request(),
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
    let mut clients = HttpClients::new(support::policy(Verification::Enabled, None));
    let mut logging = Logging::default();
    let mut call = Call::new(serving.address);
    call.params = support::params(r#"{"custom_llm_provider": "nope"}"#);

    let output = async_get_interaction(&fixture, &mut clients, &mut logging, None, call.request())
        .await
        .unwrap();

    assert_eq!(output, "{}");
    assert!(
        clients
            .async_clients
            .keys()
            .any(|key| key.provider == ProviderId::Gemini)
    );
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_get_interaction_passes_a_failing_status_to_the_response_transform() {
    let serving = support::serve_once(
        support::reply("500 Internal Server Error", &[], "boom"),
        None,
    );
    let mut fixture = Fixture::default();
    fixture.refuse_status = Some(500);
    let mut clients = HttpClients::new(support::policy(Verification::Enabled, None));
    let mut logging = Logging::default();

    let error = async_get_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        Call::new(serving.address).request(),
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        GetInteractionError::Provider(FixtureError::Refused(500))
    ));
    serving.request.join().unwrap();
}
