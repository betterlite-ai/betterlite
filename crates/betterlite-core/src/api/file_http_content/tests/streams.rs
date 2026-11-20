use std::iter::repeat_with;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::Serving;
use betterlite_http_fixture::server::serve;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::stream::{FileContentStreamingResult, StreamExit};
use futures_util::StreamExt as _;
use http::Method;
use indexmap::IndexMap;
use rstest::rstest;

use super::support::{
    Events, Fixture, HeaderPairs, IMPATIENT, MockScenario, ProxiedSource, Source, Wire,
    caller_headers, error_headers, gated, locked, params, provider_failure, provider_headers,
    recording_config, request, retrieve_call, streaming,
};
use crate::api::file_http_content::config::{Headers, QueryParams};
use crate::api::file_http_content::error::ContentError;
use crate::api::file_http_content::service::ContentDeps;
use crate::api::file_http_content::service::streaming::retrieve_content_streaming;

const ERROR_PAIRS: [(&str, &str); 2] = [("content-length", "4"), ("content-type", "text/plain")];

async fn drained(result: &mut FileContentStreamingResult) -> Vec<Vec<u8>> {
    let mut chunks = Vec::new();
    while let Some(chunk) = result.stream.next().await {
        chunks.push(chunk.expect("the body is read"));
    }
    chunks
}

fn assert_sent_stream(scenario: &MockScenario) {
    let sent = scenario.wire.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, Method::GET);
    assert_eq!(
        sent[0].url.as_str(),
        "http://mock.test/v1/files/f/content?x=new&y=2&p=1"
    );
    assert!(sent[0].query.is_empty());
    assert_eq!(sent[0].timeout, Some(Duration::from_secs(2)));
}

fn assert_stream_transformed(scenario: &MockScenario) {
    let streams = locked(&scenario.config.streams);
    assert_eq!(streams.len(), 1);
    assert_eq!(
        streams[0].request_url,
        "http://mock.test/v1/files/f/content?x=new&y=2&p=1"
    );
    assert_eq!(streams[0].chunk_size.get(), 3);
    assert_eq!(streams[0].logged_file_id.as_deref(), Some("f"));
    assert_eq!(
        streams[0]
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/octet-stream")
    );
}

fn assert_decoded_stream_headers(scenario: &MockScenario) {
    let streams = locked(&scenario.config.streams);
    assert_eq!(streams.len(), 1);
    let headers = &streams[0].headers;
    assert!(headers.get("content-encoding").is_none());
    assert!(headers.get("content-length").is_none());
    assert_eq!(headers.get_all("x-repeat").iter().count(), 2);
    let content_type = headers
        .get("content-type")
        .and_then(|value| value.to_str().ok());
    assert_eq!(content_type, Some("application/octet-stream"));
    let disposition = headers
        .get("content-disposition")
        .and_then(|value| value.to_str().ok());
    assert_eq!(disposition, Some("attachment; filename=\"a.bin\""));
}

async fn proxied_streaming(
    delay: Duration,
    limits: TransportLimits,
    timeout: Option<Duration>,
) -> (Result<FileContentStreamingResult, ContentError>, Serving) {
    let answer = reply(
        "200 OK",
        &[("content-type", "application/octet-stream")],
        "late",
    );
    let serving = serve(answer, delay).expect("the fixture serves");
    let events = Events::new();
    let config = recording_config(
        &format!("http://{}/v1/files/f/content", serving.address),
        QueryParams::new(),
        provider_headers(),
        &events,
    );
    let source = ProxiedSource::new(limits);
    let deps = ContentDeps {
        config: &config,
        source: &source,
    };
    let litellm = params("http://unused.test");
    let headers = Headers::new();
    let content_request = request("f", IndexMap::new());
    let mut fixture = Fixture::new("file_content");
    let call = retrieve_call(&content_request, &litellm, &headers, timeout);
    let prepared = fixture
        .prepare(&deps, &call, &events)
        .expect("the request prepares");
    let chunk = NonZeroUsize::new(3).expect("3 is not zero");
    let result = retrieve_content_streaming(prepared, chunk, &mut fixture.state).await;
    (result, serving)
}

#[tokio::test]
async fn streaming_prepared_query_overrides_the_url_and_returns_before_the_body() {
    let drops = Arc::new(AtomicUsize::new(0));
    let (sender, body) = gated(&drops, 4);
    let mut scenario = MockScenario::new(
        streaming(200, &[("content-type", "application/octet-stream")], body),
        "http://mock.test/v1/files/f/content?x=old&y=2",
        QueryParams::from([
            (String::from("x"), String::from("new")),
            (String::from("p"), String::from("1")),
        ]),
    );
    scenario.timeout = Some(Duration::from_secs(2));
    let chunk = NonZeroUsize::new(3).expect("3 is not zero");
    let (_, answer) = scenario.streaming(chunk).await;
    let mut result = answer.expect("the stream opens before a byte exists");
    let exit = Arc::clone(&result.exit);
    assert_sent_stream(&scenario);
    assert_stream_transformed(&scenario);
    assert_eq!(
        result.headers.get("x-transform").map(String::as_str),
        Some("recorded")
    );
    sender
        .send(Ok(b"abcdef".to_vec()))
        .await
        .expect("the chunk is queued");
    sender
        .send(Ok(b"gh".to_vec()))
        .await
        .expect("the chunk is queued");
    drop(sender);
    let chunks = drained(&mut result).await;
    assert_eq!(
        chunks,
        vec![b"abc".to_vec(), b"def".to_vec(), b"gh".to_vec()]
    );
    assert_eq!(exit.get(), Some(&StreamExit::Completed));
    drop(result);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn streaming_null_timeout_uses_the_client_read_timeout() {
    let (result, serving) = proxied_streaming(Duration::from_millis(500), IMPATIENT, None).await;
    let failure = provider_failure(result.err().expect("the client read timeout fires"));
    assert_eq!(failure.status_code, 500);
    assert!(failure.status_code_is_synthesized);
    assert!(failure.message.contains("read timeout"));
    assert!(!failure.message.starts_with("litellm.Timeout"));
    assert!(failure.headers.is_empty());
    let sent = serving.finish().expect("the fixture answers");
    assert_eq!(sent.line, "GET /v1/files/f/content HTTP/1.1");
}

#[tokio::test]
async fn streaming_call_timeout_bounds_the_transfer() {
    let (result, serving) = proxied_streaming(
        Duration::from_millis(500),
        TransportLimits::DEFAULT,
        Some(Duration::from_millis(100)),
    )
    .await;
    let failure = provider_failure(result.err().expect("the call timeout fires"));
    assert_eq!(failure.status_code, 500);
    assert!(failure.status_code_is_synthesized);
    assert!(failure.message.contains("total deadline"));
    let sent = serving.finish().expect("the fixture answers");
    assert_eq!(sent.line, "GET /v1/files/f/content HTTP/1.1");
}

#[tokio::test]
async fn streaming_answer_within_the_client_timeout_is_read() {
    let (answer, serving) =
        proxied_streaming(Duration::from_millis(100), TransportLimits::DEFAULT, None).await;
    let mut result = answer.expect("the stream opens");
    let chunks = drained(&mut result).await;
    assert_eq!(chunks, vec![b"lat".to_vec(), b"e".to_vec()]);
    assert_eq!(result.exit.get(), Some(&StreamExit::Completed));
    serving.finish().expect("the fixture answers");
}

async fn gated_error_body(
    drops: &Arc<AtomicUsize>,
    status: u16,
    headers: &HeaderPairs,
    chunks: Vec<Result<Vec<u8>, HttpError>>,
) -> (
    MockScenario,
    Result<FileContentStreamingResult, ContentError>,
) {
    let (sender, body) = gated(drops, chunks.len());
    for chunk in chunks {
        sender.send(chunk).await.expect("the chunk is queued");
    }
    drop(sender);
    let scenario = MockScenario::new(
        streaming(status, headers, body),
        "http://mock.test/v1/files/f/content",
        QueryParams::new(),
    );
    let chunk = NonZeroUsize::new(3).expect("3 is not zero");
    let (_, result) = scenario.streaming(chunk).await;
    (scenario, result)
}

#[tokio::test]
async fn streaming_error_body_is_lossy_and_closed_once() {
    let drops = Arc::new(AtomicUsize::new(0));
    let chunks = vec![Ok(b"bad".to_vec()), Ok(vec![0xFF])];
    let (scenario, result) = gated_error_body(&drops, 400, &ERROR_PAIRS, chunks).await;
    let failure = provider_failure(result.err().expect("a 400 is a provider error"));
    assert_eq!(failure.status_code, 400);
    assert!(!failure.status_code_is_synthesized);
    assert_eq!(failure.message, "bad\u{fffd}");
    assert_eq!(failure.headers, error_headers(&ERROR_PAIRS));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(locked(&scenario.config.streams).is_empty());
    assert_eq!(scenario.events.recorded(), vec!["log", "send"]);
    assert_eq!(
        scenario.config.errors(),
        vec![(
            String::from("bad\u{fffd}"),
            400_u16,
            caller_headers(&ERROR_PAIRS)
        )]
    );
}

#[tokio::test]
async fn streaming_error_body_keeps_the_text_read_before_a_failure() {
    let drops = Arc::new(AtomicUsize::new(0));
    let chunks = vec![Ok(b"part".to_vec()), Err(HttpError::Timeout)];
    let (_, result) = gated_error_body(&drops, 400, &ERROR_PAIRS, chunks).await;
    let failure = provider_failure(result.err().expect("a 400 is a provider error"));
    assert_eq!(failure.status_code, 400);
    assert!(!failure.status_code_is_synthesized);
    assert_eq!(failure.message, "part");
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn streaming_error_body_reads_at_most_64_kib() {
    let drops = Arc::new(AtomicUsize::new(0));
    let chunks: Vec<Result<Vec<u8>, HttpError>> =
        repeat_with(|| Ok(vec![b'a'; 1024])).take(100).collect();
    let headers = [("content-length", "102400")];
    let (_, result) = gated_error_body(&drops, 413, &headers, chunks).await;
    let failure = provider_failure(result.err().expect("a 413 is a provider error"));
    assert_eq!(failure.status_code, 413);
    assert_eq!(failure.message.len(), 0x0001_0000);
    assert!(failure.message.bytes().all(|byte| byte == b'a'));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[derive(Debug, Clone, Copy)]
enum ExitCase {
    Completed,
    Failed,
    Cancelled,
}

#[rstest]
#[case::completed(ExitCase::Completed)]
#[case::failed(ExitCase::Failed)]
#[case::cancelled(ExitCase::Cancelled)]
#[tokio::test]
async fn stream_exit_closes_the_body_once(#[case] case: ExitCase) {
    let events = Events::new();
    let drops = Arc::new(AtomicUsize::new(0));
    let (sender, body) = gated(&drops, 2);
    let wire = Wire::answering(streaming(200, &[], body), &events);
    let source = Source::new(wire.clone());
    let config = recording_config(
        "http://mock.test/v1/files/f/content",
        QueryParams::new(),
        provider_headers(),
        &events,
    );
    let deps = ContentDeps {
        config: &config,
        source: &source,
    };
    let litellm = params("http://mock.test/v1");
    let headers = Headers::new();
    let content_request = request("f", IndexMap::new());
    let mut fixture = Fixture::new("file_content");
    let call = retrieve_call(&content_request, &litellm, &headers, None);
    let prepared = fixture
        .prepare(&deps, &call, &events)
        .expect("the request prepares");
    let size = NonZeroUsize::new(2).expect("2 is not zero");
    let result = retrieve_content_streaming(prepared, size, &mut fixture.state)
        .await
        .expect("the stream opens");
    let exit = Arc::clone(&result.exit);
    match case {
        ExitCase::Completed => {
            sender
                .send(Ok(b"abc".to_vec()))
                .await
                .expect("the chunk is queued");
            drop(sender);
            let mut stream = result.stream;
            let mut chunks = Vec::new();
            while let Some(chunk) = stream.next().await {
                chunks.push(chunk.expect("the body is read"));
            }
            assert_eq!(chunks, vec![b"ab".to_vec(), b"c".to_vec()]);
            drop(stream);
            assert_eq!(exit.get(), Some(&StreamExit::Completed));
        }
        ExitCase::Failed => {
            sender
                .send(Ok(b"ab".to_vec()))
                .await
                .expect("the chunk is queued");
            sender
                .send(Err(HttpError::Timeout))
                .await
                .expect("the failure is queued");
            drop(sender);
            let mut stream = result.stream;
            let first = stream
                .next()
                .await
                .expect("a chunk")
                .expect("the chunk arrives");
            assert_eq!(first, b"ab".to_vec());
            let Err(failure) = stream.next().await.expect("an error") else {
                panic!("the body fails");
            };
            assert!(matches!(failure, FilesError::Body { .. }));
            assert!(stream.next().await.is_none());
            drop(stream);
            assert_eq!(
                exit.get(),
                Some(&StreamExit::Failed(String::from("the request timed out")))
            );
        }
        ExitCase::Cancelled => {
            drop(result);
            assert_eq!(exit.get(), Some(&StreamExit::Closed));
        }
    }
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn streaming_transform_receives_the_decoded_headers() {
    let drops = Arc::new(AtomicUsize::new(0));
    let (sender, body) = gated(&drops, 1);
    let scenario = MockScenario::new(
        streaming(
            200,
            &[
                ("content-encoding", "gzip"),
                ("content-length", "10"),
                ("content-type", "application/octet-stream"),
                ("content-disposition", "attachment; filename=\"a.bin\""),
                ("x-repeat", "one"),
                ("x-repeat", "two"),
            ],
            body,
        ),
        "http://mock.test/v1/files/f/content",
        QueryParams::new(),
    );
    let chunk = NonZeroUsize::new(3).expect("3 is not zero");
    let (_, answer) = scenario.streaming(chunk).await;
    let result = answer.expect("the stream opens");
    assert_decoded_stream_headers(&scenario);
    assert!(result.headers.contains_key("content-disposition"));
    assert!(result.headers.contains_key("content-type"));
    assert!(!result.headers.contains_key("content-encoding"));
    assert_eq!(
        result.headers.get("x-repeat").map(String::as_str),
        Some("one, two")
    );
    drop(result);
    drop(sender);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn streaming_non_error_status_reaches_the_transform() {
    let drops = Arc::new(AtomicUsize::new(0));
    let (sender, body) = gated(&drops, 1);
    let scenario = MockScenario::new(
        streaming(302, &[("location", "http://elsewhere.test/file")], body),
        "http://mock.test/v1/files/f/content",
        QueryParams::new(),
    );
    let chunk = NonZeroUsize::new(3).expect("3 is not zero");
    let (_, answer) = scenario.streaming(chunk).await;
    let result = answer.expect("a 3xx is an answer");
    assert_eq!(
        result.headers.get("location").map(String::as_str),
        Some("http://elsewhere.test/file")
    );
    assert_eq!(locked(&scenario.config.streams).len(), 1);
    assert!(scenario.config.errors().is_empty());
    drop(result);
    drop(sender);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
