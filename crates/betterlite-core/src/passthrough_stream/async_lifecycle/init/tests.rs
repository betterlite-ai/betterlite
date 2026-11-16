#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};

use http::HeaderValue;
use rstest::rstest;

use crate::passthrough_stream::error::PassthroughStreamError;
use crate::passthrough_stream::tests::{
    Fixture, ReplyFixture, SourceLog, chunk_steps, failed_send, script,
};

#[tokio::test]
async fn init_stores_status_headers_and_source_on_success() {
    let reply = ReplyFixture::ok(script(chunk_steps(&["one"])));
    let source = reply.log();
    let mut fixture = Fixture::awaiting(reply);
    assert!(!fixture.stream.initialized);
    fixture
        .stream
        .init()
        .await
        .expect("the 200 reply initializes");
    assert!(fixture.stream.initialized);
    assert!(fixture.stream.pending.is_none());
    assert!(fixture.stream.source.is_some());
    assert_eq!(fixture.stream.status_code().expect("initialized"), 200);
    assert_eq!(
        fixture
            .stream
            .headers()
            .expect("initialized")
            .get("x-upstream"),
        Some(&HeaderValue::from_static("yes"))
    );
    let log = source.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(log.body_reads, 0);
    assert_eq!(log.closed, 0);
}

#[tokio::test]
async fn init_reads_and_closes_an_error_response_then_errors() {
    let reply = ReplyFixture::ok(script(Vec::new()))
        .with_status(404)
        .with_body("denied");
    let source = reply.log();
    let mut fixture = Fixture::awaiting(reply);
    let error = fixture.stream.init().await.expect_err("404 cannot stream");
    let message = error.to_string();
    assert_eq!(message, "the upstream answered with status 404");
    let PassthroughStreamError::UpstreamStatus(status) = error else {
        panic!("expected the upstream status to be reported");
    };
    assert_eq!(status.status_code, 404);
    assert_eq!(status.body, b"denied".to_vec());
    assert_eq!(
        status.headers.get("x-upstream"),
        Some(&HeaderValue::from_static("yes"))
    );
    assert!(fixture.stream.initialized);
    assert_eq!(fixture.stream.status_code().expect("initialized"), 404);
    let log = source.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(log.body_reads, 1);
    assert_eq!(log.closed, 1);
}

#[tokio::test]
async fn init_is_idempotent_after_success() {
    let mut fixture = Fixture::awaiting(ReplyFixture::ok(script(Vec::new())));
    fixture
        .stream
        .init()
        .await
        .expect("the 200 reply initializes");
    fixture
        .stream
        .init()
        .await
        .expect("a second await returns the same stream");
    assert!(fixture.stream.source.is_some());
    assert_eq!(fixture.stream.status_code().expect("initialized"), 200);
}

#[tokio::test]
async fn init_of_a_consumed_send_reports_it_was_awaited() {
    let mut fixture =
        Fixture::from_pending(failed_send(), Arc::new(Mutex::new(SourceLog::default())));
    let error = fixture.stream.init().await.expect_err("the send fails");
    assert_eq!(
        error.to_string(),
        "the passthrough request could not be sent"
    );
    let error = fixture
        .stream
        .init()
        .await
        .expect_err("the send is consumed");
    assert_eq!(
        error.to_string(),
        "the async passthrough stream cannot be awaited twice; its send was already consumed"
    );
    assert!(matches!(error, PassthroughStreamError::AlreadyAwaited));
    assert!(!fixture.stream.initialized);
}

#[rstest]
#[case(199, false)]
#[case(200, true)]
#[case(299, true)]
#[case(300, false)]
#[case(404, false)]
#[tokio::test]
async fn init_streams_only_2xx_statuses(#[case] status: u16, #[case] streams: bool) {
    let reply = ReplyFixture::ok(script(chunk_steps(&["one"]))).with_status(status);
    let mut fixture = Fixture::awaiting(reply);
    let result = fixture.stream.init().await;
    assert_eq!(result.is_ok(), streams);
    assert_eq!(fixture.stream.source.is_some(), streams);
}

#[tokio::test]
async fn init_reports_an_unreadable_error_body_as_empty() {
    let reply = ReplyFixture::ok(script(Vec::new()))
        .with_status(500)
        .failing_body();
    let source = reply.log();
    let mut fixture = Fixture::awaiting(reply);
    let error = fixture.stream.init().await.expect_err("500 cannot stream");
    let PassthroughStreamError::UpstreamStatus(status) = error else {
        panic!("expected the upstream status to be reported");
    };
    assert_eq!(status.status_code, 500);
    assert!(status.body.is_empty());
    let log = source.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(log.body_reads, 1);
    assert_eq!(log.closed, 1);
}

#[tokio::test]
async fn init_keeps_the_status_when_the_error_close_fails() {
    let reply = ReplyFixture::ok(script(Vec::new()))
        .with_status(503)
        .with_body("busy")
        .failing_close();
    let mut fixture = Fixture::awaiting(reply);
    let error = fixture.stream.init().await.expect_err("503 cannot stream");
    let PassthroughStreamError::UpstreamStatus(status) = error else {
        panic!("expected the upstream status to be reported");
    };
    assert_eq!(status.status_code, 503);
    assert_eq!(status.body, b"busy".to_vec());
}
