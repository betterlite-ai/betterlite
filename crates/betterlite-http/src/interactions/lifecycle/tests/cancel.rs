use std::time::Duration;

use reqwest::blocking::ClientBuilder as BlockingClientBuilder;

use super::support::{
    Call, Fixture, FixtureError, api_key_header, async_handler, build_handler, dead_address,
    serve_with_body, sync_handler,
};
use crate::interactions::base::RequestFailure;
use crate::interactions::base::tests::support as base;
use crate::interactions::lifecycle::LifecycleError;

#[test]
fn cancel_interaction_posts_the_transformed_body_as_json() {
    let serving = serve_with_body(
        base::reply(
            "200 OK",
            &[("x-request-id", "r2")],
            "{\"status\":\"cancelled\"}",
        ),
        None,
    );
    let fixture = Fixture {
        headers: api_key_header(),
        cancel_data: base::params(r#"{"action": "cancel", "reason": "user"}"#),
        ..Fixture::new()
    };
    let mut handler = sync_handler(&fixture);

    let output = handler
        .cancel_interaction(&Call::new(serving.address).request())
        .unwrap();

    assert_eq!(output, "{\"status\":\"cancelled\"}");
    let received = serving.request.join().unwrap();
    assert!(
        received
            .head
            .starts_with("POST /v1beta/interactions/int_1:cancel HTTP/1.1"),
        "{}",
        received.head
    );
    assert!(
        received.head.contains("content-type: application/json\r\n"),
        "{}",
        received.head
    );
    assert_eq!(received.body, r#"{"action":"cancel","reason":"user"}"#);
    assert_eq!(
        fixture.calls(),
        vec![
            "environment||0|1".to_owned(),
            "cancel_request|int_1|1|1".to_owned(),
            "cancel_response|200|int_1|r2".to_owned(),
        ]
    );
}

#[test]
fn cancel_interaction_turns_a_failing_status_into_the_error_class() {
    let serving = serve_with_body(base::reply("500 Internal Server Error", &[], "boom"), None);
    let fixture = Fixture::new();
    let mut handler = sync_handler(&fixture);

    let error = handler
        .cancel_interaction(&Call::new(serving.address).request())
        .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Provider(FixtureError::Refused(500))
    ));
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| !call.starts_with("cancel_response|"))
    );
    serving.request.join().unwrap();
}

#[test]
fn cancel_interaction_turns_a_3xx_answer_into_the_error_class() {
    let serving = serve_with_body(
        base::reply("300 Multiple Choices", &[], "{\"status\":\"cancelled\"}"),
        None,
    );
    let fixture = Fixture::new();
    let mut handler = sync_handler(&fixture);

    let error = handler
        .cancel_interaction(&Call::new(serving.address).request())
        .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Provider(FixtureError::Refused(300))
    ));
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| !call.starts_with("cancel_response|"))
    );
    serving.request.join().unwrap();
}

#[test]
fn cancel_interaction_sends_the_given_timeout() {
    let serving = base::serve_once(
        base::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(500)),
    );
    let fixture = Fixture::new();
    let mut handler = sync_handler(&fixture);
    let mut call = Call::new(serving.address);
    call.timeout = Some(Duration::from_millis(50));
    call.request_timeout = Duration::from_secs(10);

    let error = handler.cancel_interaction(&call.request()).unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Transport(RequestFailure::Transport(source)) if source.is_timeout()
    ));
    serving.request.join().unwrap();
}

#[test]
fn cancel_interaction_falls_back_to_the_request_timeout() {
    let serving = base::serve_once(
        base::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(500)),
    );
    let fixture = Fixture::new();
    let mut handler = sync_handler(&fixture);
    let mut call = Call::new(serving.address);
    call.request_timeout = Duration::from_millis(50);

    let error = handler.cancel_interaction(&call.request()).unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Transport(RequestFailure::Transport(source)) if source.is_timeout()
    ));
    serving.request.join().unwrap();
}

#[test]
fn cancel_interaction_uses_a_supplied_client() {
    let serving = base::serve_once(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::new();
    let supplied = BlockingClientBuilder::new().build().unwrap();
    let mut handler = build_handler(&fixture, Some(supplied));

    let output = handler
        .cancel_interaction(&Call::new(serving.address).request())
        .unwrap();

    assert_eq!(output, "{}");
    assert!(handler.clients.sync_clients.is_empty());
    serving.request.join().unwrap();
}

#[test]
fn cancel_interaction_passes_a_transport_failure_through_the_error_handler() {
    let fixture = Fixture::new();
    let mut handler = sync_handler(&fixture);

    let error = handler
        .cancel_interaction(&Call::new(dead_address()).request())
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

#[tokio::test]
async fn async_cancel_interaction_posts_the_transformed_body_as_json() {
    let serving = serve_with_body(
        base::reply("200 OK", &[], "{\"status\":\"cancelled\"}"),
        None,
    );
    let fixture = Fixture {
        cancel_data: base::params(r#"{"action": "cancel", "reason": "user"}"#),
        ..Fixture::new()
    };
    let mut handler = async_handler(&fixture);

    let output = handler
        .async_cancel_interaction(&Call::new(serving.address).request())
        .await
        .unwrap();

    assert_eq!(output, "{\"status\":\"cancelled\"}");
    let received = serving.request.join().unwrap();
    assert!(
        received
            .head
            .starts_with("POST /v1beta/interactions/int_1:cancel HTTP/1.1"),
        "{}",
        received.head
    );
    assert_eq!(received.body, r#"{"action":"cancel","reason":"user"}"#);
}

#[tokio::test]
async fn async_cancel_interaction_sends_the_given_timeout() {
    let serving = base::serve_once(
        base::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(500)),
    );
    let fixture = Fixture::new();
    let mut handler = async_handler(&fixture);
    let mut call = Call::new(serving.address);
    call.timeout = Some(Duration::from_millis(50));
    call.request_timeout = Duration::from_secs(10);

    let error = handler
        .async_cancel_interaction(&call.request())
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Transport(RequestFailure::Transport(source)) if source.is_timeout()
    ));
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_cancel_interaction_falls_back_to_the_request_timeout() {
    let serving = base::serve_once(
        base::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(500)),
    );
    let fixture = Fixture::new();
    let mut handler = async_handler(&fixture);
    let mut call = Call::new(serving.address);
    call.request_timeout = Duration::from_millis(50);

    let error = handler
        .async_cancel_interaction(&call.request())
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Transport(RequestFailure::Transport(source)) if source.is_timeout()
    ));
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_cancel_interaction_turns_a_failing_status_into_the_error_class() {
    let serving = serve_with_body(base::reply("404 Not Found", &[], "gone"), None);
    let fixture = Fixture::new();
    let mut handler = async_handler(&fixture);

    let error = handler
        .async_cancel_interaction(&Call::new(serving.address).request())
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Provider(FixtureError::Refused(404))
    ));
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| !call.starts_with("cancel_response|"))
    );
    serving.request.join().unwrap();
}
