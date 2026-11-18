#![cfg(test)]

use std::net::TcpListener;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use rstest::rstest;
use tokio_util::task::TaskTracker;

use super::{AsyncPassthroughReply, PassthroughSend, async_passthrough_request};
use crate::passthrough_stream::error::PassthroughStreamError;
use crate::passthrough_stream::tests::{
    ENDPOINT, FakeLogging, FakeSender, MODEL, PROVIDER, ReplyFixture, chunk_steps, fake_config,
    outbound_request, proxy_client, script,
};

const FIXTURE_URL: &str = "http://fixture.test/v1/chat/completions";

fn send_to<'call>(
    client: &'call dyn super::PassthroughSender,
    url: &str,
    streaming: bool,
) -> PassthroughSend<'call> {
    PassthroughSend {
        client,
        request: outbound_request(url),
        is_streaming_request: streaming,
    }
}

fn logging() -> Arc<FakeLogging> {
    Arc::new(FakeLogging::fixture())
}

#[tokio::test]
async fn non_streaming_reply_reads_the_body() {
    let sender = FakeSender::replying(ReplyFixture::ok(script(Vec::new())).with_body("pong"));
    let reply = async_passthrough_request(
        send_to(&sender, FIXTURE_URL, false),
        fake_config(),
        logging(),
        TaskTracker::new(),
    )
    .await
    .expect("the reply arrives");
    let AsyncPassthroughReply::Full(full) = reply else {
        panic!("expected a full reply");
    };
    assert_eq!(full.status_code, 200);
    assert_eq!(full.body, b"pong".to_vec());
    assert_eq!(
        full.headers.get("x-upstream"),
        Some(&http::HeaderValue::from_static("yes"))
    );
}

#[tokio::test]
async fn non_streaming_status_error_reads_the_body_first() {
    let reply_fixture = ReplyFixture::ok(script(Vec::new()))
        .with_status(404)
        .with_body("missing");
    let sender = FakeSender::replying(reply_fixture);
    let reply = async_passthrough_request(
        send_to(&sender, FIXTURE_URL, false),
        fake_config(),
        logging(),
        TaskTracker::new(),
    )
    .await;
    let error = reply.err().expect("404 is an error");
    let PassthroughStreamError::UpstreamStatus(status) = error else {
        panic!("expected the upstream status to be reported");
    };
    assert_eq!(status.status_code, 404);
    assert_eq!(status.body, b"missing".to_vec());
    assert_eq!(
        status.headers.get("x-upstream"),
        Some(&http::HeaderValue::from_static("yes"))
    );
}

#[rstest]
#[case(199, false)]
#[case(200, true)]
#[case(299, true)]
#[case(300, false)]
#[case(404, false)]
#[tokio::test]
async fn non_streaming_status_accepts_only_2xx(#[case] status: u16, #[case] accepted: bool) {
    let reply_fixture = ReplyFixture::ok(script(Vec::new()))
        .with_status(status)
        .with_body("body");
    let sender = FakeSender::replying(reply_fixture);
    let reply = async_passthrough_request(
        send_to(&sender, FIXTURE_URL, false),
        fake_config(),
        logging(),
        TaskTracker::new(),
    )
    .await;
    assert_eq!(reply.is_ok(), accepted);
}

#[tokio::test]
async fn non_streaming_body_read_failure_propagates() {
    let sender = FakeSender::replying(ReplyFixture::ok(script(Vec::new())).failing_body());
    let reply = async_passthrough_request(
        send_to(&sender, FIXTURE_URL, false),
        fake_config(),
        logging(),
        TaskTracker::new(),
    )
    .await;
    let error = reply.err().expect("the body read fails");
    assert_eq!(
        error.to_string(),
        "the upstream passthrough response body could not be read"
    );
    assert!(matches!(error, PassthroughStreamError::BodyRead(_)));
}

#[tokio::test]
async fn non_streaming_send_failure_propagates() {
    let sender = FakeSender::failing();
    let reply = async_passthrough_request(
        send_to(&sender, FIXTURE_URL, false),
        fake_config(),
        logging(),
        TaskTracker::new(),
    )
    .await;
    let error = reply.err().expect("the send fails");
    assert_eq!(
        error.to_string(),
        "the passthrough request could not be sent"
    );
    assert!(matches!(error, PassthroughStreamError::Send(_)));
}

#[rstest]
#[case(true)]
#[case(false)]
#[tokio::test]
async fn a_sync_client_answer_is_refused(#[case] streaming: bool) {
    let sender = FakeSender::immediate();
    let reply = async_passthrough_request(
        send_to(&sender, FIXTURE_URL, streaming),
        fake_config(),
        logging(),
        TaskTracker::new(),
    )
    .await;
    let error = reply.err().expect("a sync answer is not a coroutine");
    assert_eq!(error.to_string(), "Expected coroutine from async client");
    assert!(matches!(error, PassthroughStreamError::ExpectedCoroutine));
}

#[tokio::test]
async fn streaming_reply_is_initialized_and_collects_with_the_logging_target() {
    let sender = FakeSender::replying(ReplyFixture::ok(script(chunk_steps(&["one"]))));
    let logging = logging();
    let config = fake_config();
    let collected = Arc::clone(&config.collected);
    let reply = async_passthrough_request(
        send_to(&sender, FIXTURE_URL, true),
        config,
        Arc::clone(&logging),
        TaskTracker::new(),
    )
    .await
    .expect("the stream starts");
    let AsyncPassthroughReply::Stream(mut stream) = reply else {
        panic!("expected a stream");
    };
    assert!(stream.initialized);
    assert_eq!(stream.status_code().expect("initialized"), 200);
    assert_eq!(
        stream.next_chunk().await.expect("first chunk"),
        Some(b"one".to_vec())
    );
    assert_eq!(stream.next_chunk().await.expect("end"), None);
    stream.background_tasks.close();
    stream.background_tasks.wait().await;
    let flushes = logging.flushed();
    assert_eq!(flushes.len(), 1);
    assert_eq!(flushes[0].logged, "one");
    let records = collected
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].model, MODEL);
    assert_eq!(records[0].custom_llm_provider, PROVIDER);
    assert_eq!(records[0].endpoint, ENDPOINT);
}

#[tokio::test]
async fn streaming_status_error_is_reported_before_the_stream() {
    let reply_fixture = ReplyFixture::ok(script(Vec::new()))
        .with_status(500)
        .with_body("boom");
    let sender = FakeSender::replying(reply_fixture);
    let source = sender.log();
    let reply = async_passthrough_request(
        send_to(&sender, FIXTURE_URL, true),
        fake_config(),
        logging(),
        TaskTracker::new(),
    )
    .await;
    let error = reply.err().expect("500 cannot stream");
    let PassthroughStreamError::UpstreamStatus(status) = error else {
        panic!("expected the upstream status to be reported");
    };
    assert_eq!(status.status_code, 500);
    assert_eq!(status.body, b"boom".to_vec());
    let log = source.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(log.body_reads, 1);
    assert_eq!(log.closed, 1);
}

#[tokio::test]
async fn streaming_send_failure_propagates() {
    let sender = FakeSender::failing();
    let reply = async_passthrough_request(
        send_to(&sender, FIXTURE_URL, true),
        fake_config(),
        logging(),
        TaskTracker::new(),
    )
    .await;
    let error = reply.err().expect("the send fails");
    assert!(matches!(error, PassthroughStreamError::Send(_)));
}

#[tokio::test]
async fn streaming_empty_body_still_returns_a_stream() {
    let sender = FakeSender::replying(ReplyFixture::ok(script(Vec::new())));
    let logging = logging();
    let reply = async_passthrough_request(
        send_to(&sender, FIXTURE_URL, true),
        fake_config(),
        Arc::clone(&logging),
        TaskTracker::new(),
    )
    .await
    .expect("the stream starts");
    let AsyncPassthroughReply::Stream(mut stream) = reply else {
        panic!("expected a stream");
    };
    assert_eq!(stream.next_chunk().await.expect("end"), None);
    stream.background_tasks.close();
    stream.background_tasks.wait().await;
    assert!(logging.flushed().is_empty());
    assert!(!stream.flush_scheduled);
}

#[tokio::test]
async fn proxied_client_relays_a_non_streaming_reply() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let client = proxy_client(1024);
    let reply = async_passthrough_request(
        send_to(
            &client,
            &format!("http://{}/v1/chat/completions", serving.address),
            false,
        ),
        fake_config(),
        logging(),
        TaskTracker::new(),
    )
    .await
    .expect("the reply arrives");
    let AsyncPassthroughReply::Full(full) = reply else {
        panic!("expected a full reply");
    };
    assert_eq!(full.status_code, 200);
    assert_eq!(full.body, b"pong".to_vec());
    assert!(
        serving
            .finish()
            .unwrap()
            .line
            .starts_with("POST /v1/chat/completions HTTP/1.1")
    );
}

#[tokio::test]
async fn proxied_client_relays_a_streaming_reply_and_collects_chunks() {
    let serving = serve(reply("200 fixture", &[], "data: one\n\n"), Duration::ZERO).unwrap();
    let client = proxy_client(1024);
    let logging = logging();
    let config = fake_config();
    let collected = Arc::clone(&config.collected);
    let reply = async_passthrough_request(
        send_to(
            &client,
            &format!("http://{}/v1/chat/completions", serving.address),
            true,
        ),
        config,
        Arc::clone(&logging),
        TaskTracker::new(),
    )
    .await
    .expect("the stream starts");
    let AsyncPassthroughReply::Stream(mut stream) = reply else {
        panic!("expected a stream");
    };
    let mut body = Vec::new();
    while let Some(chunk) = stream.next_chunk().await.expect("the body streams") {
        body.extend_from_slice(&chunk);
    }
    assert_eq!(body, b"data: one\n\n".to_vec());
    stream.background_tasks.close();
    stream.background_tasks.wait().await;
    let collected_count = collected
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .len();
    assert_eq!(collected_count, 1);
    let flushes = logging.flushed();
    assert_eq!(flushes.len(), 1);
    assert_eq!(flushes[0].logged, "data: one");
    let sent = serving.finish().unwrap();
    assert!(sent.head.contains("POST /v1/chat/completions"));
}

#[tokio::test]
async fn proxied_client_reports_an_upstream_status_with_the_body() {
    let serving = serve(reply("502 fixture", &[], "bad gateway"), Duration::ZERO).unwrap();
    let client = proxy_client(1024);
    let reply = async_passthrough_request(
        send_to(
            &client,
            &format!("http://{}/v1/chat/completions", serving.address),
            false,
        ),
        fake_config(),
        logging(),
        TaskTracker::new(),
    )
    .await;
    let error = reply.err().expect("502 is an error");
    let PassthroughStreamError::UpstreamStatus(status) = error else {
        panic!("expected the upstream status to be reported");
    };
    assert_eq!(status.status_code, 502);
    assert_eq!(status.body, b"bad gateway".to_vec());
    assert!(
        serving
            .finish()
            .unwrap()
            .head
            .contains("POST /v1/chat/completions")
    );
}

#[tokio::test]
async fn proxied_client_caps_a_non_streaming_body_at_the_transport_limit() {
    let serving = serve(reply("200 fixture", &[], "0123456789"), Duration::ZERO).unwrap();
    let client = proxy_client(4);
    let reply = async_passthrough_request(
        send_to(
            &client,
            &format!("http://{}/v1/chat/completions", serving.address),
            false,
        ),
        fake_config(),
        logging(),
        TaskTracker::new(),
    )
    .await;
    let error = reply.err().expect("the body is over the limit");
    let PassthroughStreamError::BodyRead(inner) = error else {
        panic!("expected a body read error");
    };
    assert!(matches!(inner.as_ref(), HttpError::TooLarge { limit: 4 }));
    assert!(
        serving
            .finish()
            .unwrap()
            .head
            .contains("POST /v1/chat/completions")
    );
}

#[tokio::test]
async fn proxied_client_accepts_a_body_at_the_transport_limit() {
    let serving = serve(reply("200 fixture", &[], "0123456789"), Duration::ZERO).unwrap();
    let client = proxy_client(10);
    let reply = async_passthrough_request(
        send_to(
            &client,
            &format!("http://{}/v1/chat/completions", serving.address),
            false,
        ),
        fake_config(),
        logging(),
        TaskTracker::new(),
    )
    .await
    .expect("the body fits");
    let AsyncPassthroughReply::Full(full) = reply else {
        panic!("expected a full reply");
    };
    assert_eq!(full.body, b"0123456789".to_vec());
    assert!(
        serving
            .finish()
            .unwrap()
            .head
            .contains("POST /v1/chat/completions")
    );
}

#[tokio::test]
async fn proxied_client_reports_a_failed_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let client = proxy_client(1024);
    let url = format!("http://{address}/v1/chat/completions");
    let reply = async_passthrough_request(
        send_to(&client, &url, false),
        fake_config(),
        logging(),
        TaskTracker::new(),
    )
    .await;
    let error = reply.err().expect("the connection is refused");
    assert_eq!(
        error.to_string(),
        "the passthrough request could not be sent"
    );
    assert!(matches!(error, PassthroughStreamError::Send(_)));
}
