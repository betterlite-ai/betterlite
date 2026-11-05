use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use http::Method;
use rstest::rstest;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::super::super::bounded::CleanupError;
use super::super::super::http_client::{CleanupScope, ClientError, McpHttpClient};
use super::super::super::recording::ResponseBody as _;
use super::{FakeBody, FakeError, FakeTransport, Reply, deadline_in, reply, seconds};

fn client(replies: Vec<Reply>) -> (McpHttpClient<FakeTransport>, Arc<FakeTransport>) {
    let transport = FakeTransport::new(replies);
    let client = McpHttpClient::new(Arc::clone(&transport), TaskTracker::new());
    (client, transport)
}

fn scope(shield: bool, limit: u64) -> CleanupScope {
    CleanupScope {
        shield,
        deadline: deadline_in(limit),
        caller_cancelled: CancellationToken::new(),
    }
}

#[tokio::test(start_paused = true)]
async fn delete_inside_a_scope_reads_the_response_before_returning() {
    let body = FakeBody::new(vec![Ok(b"done".to_vec())]);
    let read = Arc::clone(&body.read);
    let (client, transport) = client(vec![reply(204, body)]);
    client.enter_cleanup(scope(true, 5));
    let response = client.send(Method::DELETE, false).await.unwrap();
    assert_eq!(response.status, 204);
    assert!(read.load(Ordering::SeqCst));
    assert_eq!(transport.sent.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn delete_outside_a_scope_is_sent_like_any_other_request() {
    let body = FakeBody::new(vec![]);
    let read = Arc::clone(&body.read);
    let (client, _) = client(vec![reply(204, body)]);
    client.send(Method::DELETE, false).await.unwrap();
    assert!(!read.load(Ordering::SeqCst));
}

#[tokio::test(start_paused = true)]
async fn delete_is_bounded_by_the_scope_deadline_and_not_recorded() {
    let mut slow = reply(204, FakeBody::new(vec![]));
    slow.delay = seconds(60);
    let (client, _) = client(vec![slow]);
    client.enter_cleanup(scope(true, 5));
    let started = Instant::now();
    let error = client.send(Method::DELETE, false).await.err().unwrap();
    assert!(matches!(
        error,
        ClientError::Cleanup(CleanupError::TimedOut(_))
    ));
    assert_eq!(started.elapsed(), seconds(5));
    assert!(client.leave_cleanup().is_empty());
}

#[tokio::test(start_paused = true)]
async fn delete_failure_surfaces_the_operation_error() {
    let failing = Reply {
        delay: Duration::ZERO,
        outcome: Err(FakeError::Boom("connect")),
    };
    let (client, _) = client(vec![failing]);
    client.enter_cleanup(scope(true, 5));
    let error = client.send(Method::DELETE, false).await.err().unwrap();
    assert!(matches!(
        error,
        ClientError::Cleanup(CleanupError::Operation(FakeError::Boom("connect")))
    ));
}

#[tokio::test(start_paused = true)]
async fn delete_cancelled_by_the_caller_still_reads_the_response() {
    let body = FakeBody::new(vec![]);
    let read = Arc::clone(&body.read);
    let (client, _) = client(vec![reply(204, body)]);
    let cleanup = scope(true, 5);
    cleanup.caller_cancelled.cancel();
    client.enter_cleanup(cleanup);
    let error = client.send(Method::DELETE, false).await.err().unwrap();
    assert!(matches!(
        error,
        ClientError::Cleanup(CleanupError::Cancelled)
    ));
    assert!(read.load(Ordering::SeqCst));
}

#[rstest]
#[case::lower_bound(400)]
#[case::forbidden(403)]
#[case::server_error(500)]
#[case::upper_bound(599)]
#[tokio::test]
async fn post_error_statuses_close_the_response_and_raise(#[case] status: u16) {
    let body = FakeBody::new(vec![]);
    let closed = Arc::clone(&body.closed);
    let (client, _) = client(vec![reply(status, body)]);
    let ClientError::Transport(error) = client.send(Method::POST, false).await.err().unwrap()
    else {
        panic!("expected a transport error");
    };
    assert_eq!(*error, FakeError::Status(status));
    assert!(closed.load(Ordering::SeqCst));
}

#[rstest]
#[case::not_found(404)]
#[case::success(200)]
#[case::redirect(302)]
#[case::below_error_range(399)]
#[case::above_error_range(600)]
#[tokio::test]
async fn post_other_statuses_are_returned_unchanged(#[case] status: u16) {
    let body = FakeBody::new(vec![]);
    let closed = Arc::clone(&body.closed);
    let (client, _) = client(vec![reply(status, body)]);
    let response = client.send(Method::POST, false).await.unwrap();
    assert_eq!(response.status, status);
    assert!(!closed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn get_error_statuses_are_returned_unchanged() {
    let (client, _) = client(vec![reply(500, FakeBody::new(vec![]))]);
    assert_eq!(client.send(Method::GET, false).await.unwrap().status, 500);
}

#[tokio::test]
async fn a_post_close_failure_replaces_the_status_error_and_is_recorded() {
    let mut body = FakeBody::new(vec![]);
    body.close_error = Some(FakeError::Boom("close"));
    let (client, _) = client(vec![reply(500, body)]);
    client.enter_cleanup(scope(true, 5));
    let ClientError::Transport(error) = client.send(Method::POST, false).await.err().unwrap()
    else {
        panic!("expected a transport error");
    };
    assert_eq!(*error, FakeError::Boom("close"));
    assert!(Arc::ptr_eq(&client.leave_cleanup()[0], &error));
}

#[tokio::test]
async fn streamed_responses_record_body_failures_inside_a_scope() {
    let body = FakeBody::new(vec![Err(FakeError::Boom("read"))]);
    let (client, transport) = client(vec![reply(200, body)]);
    client.enter_cleanup(scope(true, 5));
    let mut response = client.send(Method::GET, true).await.unwrap();
    let error = response.body.next_chunk().await.unwrap().unwrap_err();
    assert!(Arc::ptr_eq(&client.leave_cleanup()[0], &error));
    assert_eq!(transport.sent.lock().unwrap()[0], (Method::GET, true));
}

#[tokio::test]
async fn buffered_responses_are_not_wrapped() {
    let body = FakeBody::new(vec![Err(FakeError::Boom("read"))]);
    let (client, _) = client(vec![reply(200, body)]);
    client.enter_cleanup(scope(true, 5));
    let mut response = client.send(Method::GET, false).await.unwrap();
    response.body.next_chunk().await.unwrap().unwrap_err();
    assert!(client.leave_cleanup().is_empty());
}

#[rstest]
#[case::shielded_scope(Some(true), 1)]
#[case::unshielded_scope(Some(false), 0)]
#[case::no_scope(None, 0)]
#[tokio::test]
async fn send_failures_are_recorded_only_in_a_shielded_scope(
    #[case] scoped: Option<bool>,
    #[case] recorded: usize,
) {
    let failing = Reply {
        delay: Duration::ZERO,
        outcome: Err(FakeError::Boom("connect")),
    };
    let (client, _) = client(vec![failing]);
    if let Some(shield) = scoped {
        client.enter_cleanup(scope(shield, 5));
    }
    let ClientError::Transport(error) = client.send(Method::POST, false).await.err().unwrap()
    else {
        panic!("expected a transport error");
    };
    assert_eq!(*error, FakeError::Boom("connect"));
    let errors = client.leave_cleanup();
    assert_eq!(errors.len(), recorded);
    assert!(errors.iter().all(|kept| Arc::ptr_eq(kept, &error)));
}

#[tokio::test]
async fn recorded_errors_persist_after_the_scope_ends_and_across_sends() {
    let failing = || Reply {
        delay: Duration::ZERO,
        outcome: Err(FakeError::Boom("connect")),
    };
    let (client, _) = client(vec![failing(), failing(), failing()]);
    client.enter_cleanup(scope(true, 5));
    client.send(Method::POST, false).await.err().unwrap();
    client.send(Method::POST, false).await.err().unwrap();
    assert_eq!(client.leave_cleanup().len(), 2);
    client.send(Method::POST, false).await.err().unwrap();
    assert_eq!(client.leave_cleanup().len(), 2);
}
