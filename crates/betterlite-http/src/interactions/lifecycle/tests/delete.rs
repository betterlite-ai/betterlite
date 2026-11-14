use std::time::Duration;

use betterlite_config::tls::Verification;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::identity::provider_id::ProviderId;
use reqwest::blocking::ClientBuilder as BlockingClientBuilder;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::support::{
    Call, Fixture, FixtureError, api_key_header, async_handler, caller_headers, clients,
    dead_address, serve_with_body, sync_handler,
};
use crate::interactions::base::clients::HttpClients;
use crate::interactions::base::tests::support as base;
use crate::interactions::base::{ClientError, Logging, RequestFailure};
use crate::interactions::lifecycle::{InteractionsHttpHandler, LifecycleError};
use crate::proxy::base_builder;

#[test]
fn delete_interaction_deletes_the_interaction_through_the_config() {
    let serving = serve_with_body(
        base::reply("200 OK", &[("x-request-id", "r1")], "{\"id\":\"int_1\"}"),
        None,
    );
    let mut fixture = Fixture::default();
    fixture.headers = api_key_header();
    let mut call = Call::new(serving.address);
    call.params.insert(
        "custom_llm_provider".to_owned(),
        JsonValue::Text("gemini".to_owned()),
    );
    call.extra_headers = caller_headers(&[("x-caller", "c1")]);
    let mut handler = sync_handler(&fixture);

    let output = handler.delete_interaction(&call.request()).unwrap();

    assert_eq!(output, "{\"id\":\"int_1\"}");
    let received = serving.request.join().unwrap();
    let head = received.head.as_str();
    assert!(
        head.starts_with("DELETE /v1beta/interactions/int_1 HTTP/1.1"),
        "{head}"
    );
    assert!(head.contains("x-goog-api-key: key-1\r\n"), "{head}");
    assert!(received.body.is_empty(), "{}", received.body);
    assert_eq!(
        fixture.calls(),
        vec![
            "environment||1|2".to_owned(),
            "delete_request|int_1|1|2".to_owned(),
            "delete_response|200|int_1|int_1|r1".to_owned(),
        ]
    );
    let recorded = handler.logging.pre_call.as_ref().unwrap();
    assert_eq!(recorded.input, "int_1");
    assert_eq!(recorded.api_key.expose_secret(), "");
    assert_eq!(
        recorded.api_base,
        format!("http://{}/v1beta/interactions/int_1", serving.address)
    );
    assert_eq!(recorded.headers["x-goog-api-key"].expose_secret(), "key-1");
}

#[test]
fn delete_interaction_sends_no_body_when_the_transform_returns_data() {
    let serving = serve_with_body(base::reply("200 OK", &[], "{}"), None);
    let mut fixture = Fixture::default();
    fixture.delete_data = base::params(r#"{"ignored": "yes"}"#);
    let mut handler = sync_handler(&fixture);

    handler
        .delete_interaction(&Call::new(serving.address).request())
        .unwrap();

    let received = serving.request.join().unwrap();
    assert!(
        received
            .head
            .starts_with("DELETE /v1beta/interactions/int_1 HTTP/1.1"),
        "{}",
        received.head
    );
    assert!(received.body.is_empty(), "{}", received.body);
}

#[test]
fn delete_interaction_accepts_a_no_content_answer() {
    let serving = serve_with_body(base::reply("204 No Content", &[], ""), None);
    let fixture = Fixture::default();
    let mut handler = sync_handler(&fixture);

    let output = handler
        .delete_interaction(&Call::new(serving.address).request())
        .unwrap();

    assert_eq!(output, "");
    assert!(
        fixture
            .calls()
            .contains(&"delete_response|204|int_1|int_1|-".to_owned())
    );
    serving.request.join().unwrap();
}

#[test]
fn delete_interaction_turns_a_failing_status_into_the_error_class() {
    let serving = serve_with_body(
        base::reply(
            "429 Too Many Requests",
            &[("retry-after", "2")],
            "slow down",
        ),
        None,
    );
    let fixture = Fixture::default();
    let mut handler = sync_handler(&fixture);

    let error = handler
        .delete_interaction(&Call::new(serving.address).request())
        .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Provider(FixtureError::Refused(429))
    ));
    let recorded_calls = fixture.calls();
    assert_eq!(
        recorded_calls.last().map(String::as_str),
        Some("error|429|slow down|2")
    );
    assert!(
        recorded_calls
            .iter()
            .all(|recorded| !recorded.starts_with("delete_response|"))
    );
    serving.request.join().unwrap();
}

#[test]
fn delete_interaction_reads_a_non_utf8_error_body_as_replacement_text() {
    let body = b"bad \xff bytes";
    let mut answer = format!(
        "HTTP/1.1 500 Internal Server Error\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    answer.extend_from_slice(body);
    let serving = serve_with_body(answer, None);
    let fixture = Fixture::default();
    let mut handler = sync_handler(&fixture);

    let error = handler
        .delete_interaction(&Call::new(serving.address).request())
        .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Provider(FixtureError::Refused(500))
    ));
    assert_eq!(
        fixture.calls().last().map(String::as_str),
        Some("error|500|bad \u{fffd} bytes|-")
    );
    serving.request.join().unwrap();
}

#[test]
fn delete_interaction_keeps_a_transform_error_out_of_the_error_handler() {
    let serving = serve_with_body(base::reply("200 OK", &[], "{}"), None);
    let mut fixture = Fixture::default();
    fixture.fail_transform_status = Some(200);
    let mut handler = sync_handler(&fixture);

    let error = handler
        .delete_interaction(&Call::new(serving.address).request())
        .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Provider(FixtureError::Refused(200))
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
fn delete_interaction_passes_a_transport_failure_through_the_error_handler() {
    let fixture = Fixture::default();
    let mut handler = sync_handler(&fixture);

    let error = handler
        .delete_interaction(&Call::new(dead_address()).request())
        .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Transport(RequestFailure::Transport(_))
    ));
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| !call.starts_with("error|"))
    );
}

#[test]
fn delete_interaction_passes_an_invalid_url_through_the_error_handler() {
    let fixture = Fixture::default();
    let mut handler = sync_handler(&fixture);
    let mut call = Call::new(dead_address());
    call.params.insert(
        "api_base".to_owned(),
        JsonValue::Text("not a url".to_owned()),
    );

    let error = handler.delete_interaction(&call.request()).unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Transport(RequestFailure::Url(_))
    ));
}

#[test]
fn delete_interaction_does_not_follow_a_redirect() {
    let serving = serve_with_body(
        base::reply(
            "302 Found",
            &[("location", "http://127.0.0.1:1/elsewhere")],
            "",
        ),
        None,
    );
    let fixture = Fixture::default();
    let mut handler = sync_handler(&fixture);

    let output = handler
        .delete_interaction(&Call::new(serving.address).request())
        .unwrap();

    assert_eq!(output, "");
    assert!(
        fixture
            .calls()
            .contains(&"delete_response|302|int_1|int_1|-".to_owned())
    );
    serving.request.join().unwrap();
}

#[test]
fn delete_interaction_sends_the_given_timeout() {
    let serving = base::serve_once(
        base::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(500)),
    );
    let fixture = Fixture::default();
    let mut handler = sync_handler(&fixture);
    let mut call = Call::new(serving.address);
    call.timeout = Some(Duration::from_millis(50));
    call.request_timeout = Duration::from_secs(10);

    let error = handler.delete_interaction(&call.request()).unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Transport(RequestFailure::Transport(source)) if source.is_timeout()
    ));
    serving.request.join().unwrap();
}

#[test]
fn delete_interaction_falls_back_to_the_request_timeout() {
    let serving = base::serve_once(
        base::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(500)),
    );
    let fixture = Fixture::default();
    let mut handler = sync_handler(&fixture);
    let mut call = Call::new(serving.address);
    call.request_timeout = Duration::from_millis(50);

    let error = handler.delete_interaction(&call.request()).unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Transport(RequestFailure::Transport(source)) if source.is_timeout()
    ));
    serving.request.join().unwrap();
}

#[test]
fn delete_interaction_prefers_the_given_timeout_over_the_request_timeout() {
    let serving = base::serve_once(
        base::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(200)),
    );
    let fixture = Fixture::default();
    let mut handler = sync_handler(&fixture);
    let mut call = Call::new(serving.address);
    call.timeout = Some(Duration::from_secs(2));
    call.request_timeout = Duration::from_millis(50);

    let output = handler.delete_interaction(&call.request()).unwrap();

    assert_eq!(output, "{}");
    serving.request.join().unwrap();
}

#[test]
fn delete_interaction_uses_a_supplied_client() {
    let serving = base::serve_once(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::default();
    let supplied = BlockingClientBuilder::new()
        .timeout(Duration::from_millis(20))
        .build()
        .unwrap();
    let mut handler =
        InteractionsHttpHandler::new(&fixture, clients(), Logging::default(), Some(supplied));

    let output = handler
        .delete_interaction(&Call::new(serving.address).request())
        .unwrap();

    assert_eq!(output, "{}");
    assert!(handler.clients.sync_clients.is_empty());
    serving.request.join().unwrap();
}

#[test]
fn delete_interaction_builds_the_client_before_validating_headers() {
    let fixture = Fixture::default();
    let mut handler = sync_handler(&fixture);
    let mut call = Call::new(dead_address());
    call.params
        .insert("ssl_verify".to_owned(), JsonValue::Text("yes".to_owned()));

    let error = handler.delete_interaction(&call.request()).unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Client(ClientError::FieldType {
            field: "ssl_verify",
            ..
        })
    ));
    assert!(fixture.calls().is_empty());
}

#[rstest]
#[case::unsupported_scheme("https://127.0.0.1:1", "UnsupportedProxyScheme { scheme: \"https\" }")]
#[case::invalid_url("http://", "InvalidProxyUrl { name: \"http\" }")]
fn delete_interaction_refuses_a_proxy_it_cannot_use_instead_of_connecting_directly(
    #[case] proxy: &str,
    #[case] expected: &str,
) {
    let fixture = Fixture::default();
    let mut handler = InteractionsHttpHandler::new(
        &fixture,
        HttpClients::new(
            base::policy(Verification::Enabled, None),
            base::proxy_policy(&[("HTTP_PROXY", proxy)]),
        ),
        Logging::default(),
        None,
    );

    let error = handler
        .delete_interaction(&Call::new(dead_address()).request())
        .unwrap_err();

    assert!(
        matches!(&error, LifecycleError::Transport(failure) if format!("{failure:?}") == expected),
        "{error:?}"
    );
}

#[test]
fn delete_interaction_skips_the_proxy_check_for_a_supplied_client() {
    let serving = base::serve_once(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::default();
    let supplied = BlockingClientBuilder::new().build().unwrap();
    let mut handler = InteractionsHttpHandler::new(
        &fixture,
        HttpClients::new(
            base::policy(Verification::Enabled, None),
            base::proxy_policy(&[("HTTP_PROXY", "https://127.0.0.1:1")]),
        ),
        Logging::default(),
        Some(supplied),
    );

    let output = handler
        .delete_interaction(&Call::new(serving.address).request())
        .unwrap();

    assert_eq!(output, "{}");
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_delete_interaction_deletes_the_interaction_through_the_config() {
    let serving = serve_with_body(
        base::reply("200 OK", &[("x-request-id", "r1")], "{\"id\":\"int_1\"}"),
        None,
    );
    let mut fixture = Fixture::default();
    fixture.headers = api_key_header();
    let mut handler = async_handler(&fixture);

    let output = handler
        .async_delete_interaction(&Call::new(serving.address).request())
        .await
        .unwrap();

    assert_eq!(output, "{\"id\":\"int_1\"}");
    let received = serving.request.join().unwrap();
    assert!(
        received
            .head
            .starts_with("DELETE /v1beta/interactions/int_1 HTTP/1.1"),
        "{}",
        received.head
    );
    assert!(received.body.is_empty(), "{}", received.body);
    assert_eq!(
        fixture.calls(),
        vec![
            "environment||0|1".to_owned(),
            "delete_request|int_1|1|1".to_owned(),
            "delete_response|200|int_1|int_1|r1".to_owned(),
        ]
    );
    assert_eq!(handler.logging.pre_call.as_ref().unwrap().input, "int_1");
}

#[tokio::test]
async fn async_delete_interaction_builds_for_the_provider_its_caller_names() {
    let serving = base::serve_once(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::default();
    let mut handler = async_handler(&fixture);
    let mut call = Call::new(serving.address);
    call.provider = ProviderId::Openai;

    let output = handler
        .async_delete_interaction(&call.request())
        .await
        .unwrap();

    assert_eq!(output, "{}");
    let providers: Vec<ProviderId> = handler
        .clients
        .async_clients
        .keys()
        .map(|key| key.provider)
        .collect();
    assert!(providers.contains(&ProviderId::Openai));
    assert!(!providers.contains(&ProviderId::Gemini));
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_delete_interaction_turns_a_failing_status_into_the_error_class() {
    let serving = serve_with_body(base::reply("503 Service Unavailable", &[], "later"), None);
    let fixture = Fixture::default();
    let mut handler = async_handler(&fixture);

    let error = handler
        .async_delete_interaction(&Call::new(serving.address).request())
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Provider(FixtureError::Refused(503))
    ));
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| !call.starts_with("delete_response|"))
    );
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_delete_interaction_falls_back_to_the_request_timeout() {
    let serving = base::serve_once(
        base::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(500)),
    );
    let fixture = Fixture::default();
    let mut handler = async_handler(&fixture);
    let mut call = Call::new(serving.address);
    call.request_timeout = Duration::from_millis(50);

    let error = handler
        .async_delete_interaction(&call.request())
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Transport(RequestFailure::Transport(source)) if source.is_timeout()
    ));
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_delete_interaction_uses_a_supplied_client() {
    let serving = base::serve_once(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::default();
    let supplied = base_builder()
        .timeout(Duration::from_millis(20))
        .build()
        .unwrap();
    let mut handler =
        InteractionsHttpHandler::new(&fixture, clients(), Logging::default(), Some(supplied));

    let output = handler
        .async_delete_interaction(&Call::new(serving.address).request())
        .await
        .unwrap();

    assert_eq!(output, "{}");
    assert!(handler.clients.async_clients.is_empty());
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_delete_interaction_builds_the_client_before_validating_headers() {
    let fixture = Fixture::default();
    let mut handler = async_handler(&fixture);
    let mut call = Call::new(dead_address());
    call.params
        .insert("ssl_verify".to_owned(), JsonValue::Text("yes".to_owned()));

    let error = handler
        .async_delete_interaction(&call.request())
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Client(ClientError::FieldType {
            field: "ssl_verify",
            ..
        })
    ));
    assert!(fixture.calls().is_empty());
}
