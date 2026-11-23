use std::collections::BTreeMap;
use std::error::Error as _;
use std::net::TcpListener;
use std::sync::PoisonError;
use std::time::Duration;

use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_http::interactions::base::{ClientError, Params, RequestFailure};
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::{Serving, serve};
use betterlite_models::decisions::Json;
use http::header::HeaderMap;
use rstest::rstest;
use url::Url;

use super::support::*;
use crate::api::decisions_send::clients::{
    DecisionsAsyncTransport, DecisionsRequest, DecisionsSyncTransport,
};
use crate::api::decisions_send::error::{
    Attempt, DecisionsSendError, provider_error, send_failure,
};
use crate::api::vector_store_http_write::error::BaseLlmException;
use crate::api::vector_store_http_write::service::ClientKind;

fn class_of(error: &DecisionsSendError) -> &ErrorClass {
    match error {
        DecisionsSendError::Mapped { class, .. } => class,
        DecisionsSendError::Format(_) => panic!("the call was expected to fail as a mapped error"),
    }
}

// A server that answers well after the deadline, and the request that names that deadline
fn slow_request() -> (Serving, DecisionsRequest) {
    let serving = serve(reply("200 OK", &[], "{}"), Duration::from_millis(300))
        .expect("the slow server starts");
    let request = DecisionsRequest {
        url: Url::parse(&format!("http://{}/slow", serving.address))
            .expect("the fixture url parses"),
        headers: HeaderMap::new(),
        body: Vec::new(),
        timeout: Some(Duration::from_millis(20)),
    };
    (serving, request)
}

// The failure a blocking send raises when its deadline passes, taken from a real client
fn timeout_failure() -> RequestFailure {
    let (serving, request) = slow_request();
    let mut clients = real_clients();
    let failure = DecisionsSyncTransport::send_sync(&mut clients, request)
        .expect("the blocking client builds")
        .expect_err("the deadline ends the send");
    drop(serving.finish());
    failure
}

// A real connect failure: the listener closes before the client connects
fn connection_failure() -> RequestFailure {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port is free");
    let port = listener.local_addr().expect("the port is known").port();
    drop(listener);
    let mut clients = real_clients();
    let request = DecisionsRequest {
        url: Url::parse(&format!("http://127.0.0.1:{port}/v1/systemone"))
            .expect("the fixture url parses"),
        headers: HeaderMap::new(),
        body: Vec::new(),
        timeout: Some(Duration::from_millis(500)),
    };
    DecisionsSyncTransport::send_sync(&mut clients, request)
        .expect("the blocking client builds")
        .expect_err("nothing listens on the closed port")
}

// error_mapping: the provider's class, the synthesized status branch and the validation branch

#[test]
fn error_mapping_rate_limit_keeps_the_status_and_provider_message() {
    let mut fixture = Fixture::new();
    fixture.wire.answering(429, b"rate limited");
    fixture.wire.with_header("retry-after", "2");
    let decisions = systemone_call("typesafe", None);
    let error = fixture.send(&decisions).expect_err("the provider refused");
    assert!(matches!(class_of(&error), ErrorClass::RateLimit(_)));
    assert_eq!(
        error.message(),
        "litellm.RateLimitError: TypesafeException - rate limited"
    );
    assert_eq!(class_of(&error).metadata().status_code, 429);
    let seen = fixture.recording.seen_errors();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "rate limited");
    assert_eq!(seen[0].1, 429);
    assert_eq!(seen[0].2.get("retry-after"), Some(&String::from("2")));
    assert_eq!(fixture.wire.requests().len(), 1);
    let source = error.source().expect("the mapped error keeps its source");
    let base = source
        .downcast_ref::<BaseLlmException>()
        .expect("the source is the provider's exception");
    assert_eq!(
        base.headers.get("retry-after"),
        Some(&Json::Text(String::from("2")))
    );
}

#[test]
fn error_mapping_authentication() {
    let mut fixture = Fixture::new();
    fixture.wire.answering(401, b"bad key");
    let decisions = systemone_call("typesafe", None);
    let error = fixture.send(&decisions).expect_err("the provider refused");
    assert!(matches!(class_of(&error), ErrorClass::Authentication(_)));
    assert_eq!(
        error.message(),
        "litellm.AuthenticationError: TypesafeException - bad key"
    );
    assert_eq!(class_of(&error).metadata().status_code, 401);
}

#[test]
fn error_mapping_synthesized_connection_failure_is_an_api_connection_error() {
    let fixture = Fixture::new();
    let call = systemone_call("typesafe", None);
    let failure = connection_failure();
    let expected_text = failure.to_string();
    let error = send_failure(
        &fixture.config,
        &call,
        &fixture.log.redactor,
        failure,
        Attempt {
            mode: ClientKind::Sync,
            elapsed: Duration::default(),
        },
    );
    assert!(matches!(class_of(&error), ErrorClass::ApiConnection(_)));
    assert_eq!(
        error.message(),
        format!("litellm.APIConnectionError: TypesafeException - {expected_text}")
    );
    assert_eq!(class_of(&error).metadata().status_code, 500);
    assert_eq!(fixture.recording.seen_errors()[0].1, 500);
    let ErrorClass::ApiConnection(context) = class_of(&error) else {
        panic!("a connection failure is an APIConnectionError");
    };
    assert_eq!(context.model.as_deref(), Some("typesafe/model-1"));
}

#[rstest]
#[case::typesafe("typesafe", "TypesafeException")]
#[case::hosted_vllm("hosted_vllm", "Hosted_vllmException")]
#[case::strands_decider("strands_decider", "Strands_deciderException")]
fn error_mapping_synthesized_labels_upper_case_only_the_first_character(
    #[case] provider: &str,
    #[case] label: &str,
) {
    let fixture = Fixture::new();
    let call = systemone_call(provider, None);
    let error = provider_error(
        &call,
        &fixture.log.redactor,
        BaseLlmException {
            status_code: 500,
            message: String::from("boom"),
            headers: BTreeMap::new(),
            status_code_is_synthesized: true,
        },
    );
    assert_eq!(
        error.message(),
        format!("litellm.APIConnectionError: {label} - boom")
    );
    assert_eq!(class_of(&error).metadata().status_code, 500);
}

#[test]
fn error_mapping_sync_timeout_keeps_the_handler_message() {
    let mut fixture = Fixture::failing(timeout_failure());
    let mut decisions = systemone_call("typesafe", None);
    decisions.timeout = Some(Duration::from_millis(20));
    let error = fixture.send(&decisions).expect_err("the send timed out");
    assert!(matches!(class_of(&error), ErrorClass::Timeout(_)));
    assert_eq!(
        error.message(),
        "litellm.Timeout: Connection timed out after 0.02 seconds."
    );
    assert!(
        fixture.recording.seen_errors().is_empty(),
        "the client raises litellm.Timeout itself, so no provider class is asked for"
    );
}

#[tokio::test]
async fn error_mapping_async_timeout_keeps_the_handler_message() {
    let (serving, request) = slow_request();
    let mut clients = real_clients();
    let failure = DecisionsAsyncTransport::send_async(&mut clients, &Params::new(), request)
        .expect("the async client builds")
        .await
        .expect_err("the deadline ends the send");
    drop(serving.finish());
    let mut fixture = Fixture::failing(failure);
    let mut decisions = systemone_call("typesafe", None);
    decisions.timeout = Some(Duration::from_millis(20));
    let error = fixture
        .asend(&decisions)
        .await
        .expect_err("the send timed out");
    assert!(matches!(class_of(&error), ErrorClass::Timeout(_)));
    assert!(
        error
            .message()
            .starts_with("litellm.Timeout: Connection timed out. Timeout passed=0.02, time taken="),
        "the async handler names the time taken, got {}",
        error.message()
    );
    assert!(fixture.recording.seen_errors().is_empty());
}

#[test]
fn error_mapping_invalid_answer_goes_through_the_status_mapping() {
    let mut fixture = Fixture::new();
    *fixture
        .config
        .transform
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Transform::Invalid(String::from(
        "Decisions provider 'typesafe' returned an unexpected response: bad answers",
    ));
    let decisions = systemone_call("typesafe", None);
    let error = fixture
        .send(&decisions)
        .expect_err("the answer does not validate");
    assert!(matches!(class_of(&error), ErrorClass::InternalServer(_)));
    assert_eq!(
        error.message(),
        "litellm.InternalServerError: TypesafeException - Decisions provider 'typesafe' returned an unexpected response: bad answers"
    );
    assert_eq!(
        class_of(&error).metadata().error_type,
        WireField::Text(String::from("internal_server_error"))
    );
    assert!(
        fixture.recording.seen_errors().is_empty(),
        "the transform raises its own class, not the transport conversion"
    );
}

#[test]
fn error_mapping_malformed_answer_is_an_api_connection_error() {
    let mut fixture = Fixture::new();
    *fixture
        .config
        .transform
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Transform::Malformed;
    let decisions = systemone_call("typesafe", None);
    let error = fixture
        .send(&decisions)
        .expect_err("the answer does not parse");
    assert!(matches!(class_of(&error), ErrorClass::ApiConnection(_)));
    assert!(
        error
            .message()
            .starts_with("litellm.APIConnectionError: TypesafeException - "),
        "a malformed answer is a plain exception, mapped by exception_type"
    );
}

#[test]
fn error_mapping_client_selection_failure_is_an_api_connection_error() {
    let mut fixture = Fixture::new();
    let wire = fixture.wire.clone();
    fixture.clients = FakeClients::failing(
        &fixture.recording,
        wire,
        ClientError::UnknownProvider {
            provider: String::from("nope"),
        },
    );
    let decisions = systemone_call("typesafe", None);
    let error = fixture
        .send(&decisions)
        .expect_err("the client cannot be built");
    assert!(matches!(class_of(&error), ErrorClass::ApiConnection(_)));
    assert!(fixture.wire.requests().is_empty(), "no request is sent");
}
