use std::time::Duration;

use reqwest::blocking::ClientBuilder as BlockingClientBuilder;

use super::support::{
    Call, Fixture, FixtureError, api_key_header, async_handler, build_handler, dead_address,
    sync_handler,
};
use crate::interactions::base::RequestFailure;
use crate::interactions::base::tests::support as base;
use crate::interactions::lifecycle::LifecycleError;
use betterlite_http_fixture::server::serve;

#[test]
fn cancel_interaction_posts_the_transformed_body_as_json() {
    let serving = serve(
        base::reply(
            "200 OK",
            &[("x-request-id", "r2")],
            "{\"status\":\"cancelled\"}",
        ),
        Duration::ZERO,
    )
    .unwrap();
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
    let received = serving.finish().unwrap();
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
    let serving = serve(
        base::reply("500 Internal Server Error", &[], "boom"),
        Duration::ZERO,
    )
    .unwrap();
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
    serving.finish().unwrap();
}

#[test]
fn cancel_interaction_turns_a_3xx_answer_into_the_error_class() {
    let serving = serve(
        base::reply("300 Multiple Choices", &[], "{\"status\":\"cancelled\"}"),
        Duration::ZERO,
    )
    .unwrap();
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
    serving.finish().unwrap();
}

#[test]
fn cancel_interaction_sends_the_given_timeout() {
    let serving = serve(base::reply("200 OK", &[], "{}"), Duration::from_millis(500)).unwrap();
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
    serving.finish().unwrap();
}

#[test]
fn cancel_interaction_falls_back_to_the_request_timeout() {
    let serving = serve(base::reply("200 OK", &[], "{}"), Duration::from_millis(500)).unwrap();
    let fixture = Fixture::new();
    let mut handler = sync_handler(&fixture);
    let mut call = Call::new(serving.address);
    call.request_timeout = Duration::from_millis(50);

    let error = handler.cancel_interaction(&call.request()).unwrap_err();

    assert!(matches!(
        error,
        LifecycleError::Transport(RequestFailure::Transport(source)) if source.is_timeout()
    ));
    serving.finish().unwrap();
}

#[test]
fn cancel_interaction_uses_a_supplied_client() {
    let serving = serve(base::reply("200 OK", &[], "{}"), Duration::ZERO).unwrap();
    let fixture = Fixture::new();
    let supplied = BlockingClientBuilder::new().build().unwrap();
    let mut handler = build_handler(&fixture, Some(supplied));

    let output = handler
        .cancel_interaction(&Call::new(serving.address).request())
        .unwrap();

    assert_eq!(output, "{}");
    assert!(handler.clients.sync_clients.is_empty());
    serving.finish().unwrap();
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
    let serving = serve(
        base::reply("200 OK", &[], "{\"status\":\"cancelled\"}"),
        Duration::ZERO,
    )
    .unwrap();
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
    let received = serving.finish().unwrap();
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
    let serving = serve(base::reply("200 OK", &[], "{}"), Duration::from_millis(500)).unwrap();
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
    serving.finish().unwrap();
}

#[tokio::test]
async fn async_cancel_interaction_falls_back_to_the_request_timeout() {
    let serving = serve(base::reply("200 OK", &[], "{}"), Duration::from_millis(500)).unwrap();
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
    serving.finish().unwrap();
}

#[tokio::test]
async fn async_cancel_interaction_turns_a_failing_status_into_the_error_class() {
    let serving = serve(base::reply("404 Not Found", &[], "gone"), Duration::ZERO).unwrap();
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
    serving.finish().unwrap();
}
