use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::decisions::Json;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::stream::{FileContentStreamingResult, StreamExit};
use futures_util::StreamExt as _;
use http::Method;
use indexmap::IndexMap;
use rstest::rstest;

use super::support::{
    Events, Fixture, IMPATIENT, ProxiedSource, Source, Wire, gated, locked, params,
    provider_headers, recording_config, request, retrieve_call, streaming,
};
use crate::api::file_http_content::config::{FileContentConfig, Headers, QueryParams};
use crate::api::file_http_content::error::ContentError;
use crate::api::file_http_content::service::streaming::retrieve_content_streaming;
use crate::api::file_http_content::service::{ClientSource, ContentDeps, RetrieveCall};
use crate::api::vector_store_http_write::error::HeaderMap as ErrorHeaders;

async fn fetch_stream<'deps, C, S>(
    fixture: &mut Fixture,
    deps: &'deps ContentDeps<'deps, C, S>,
    call: &RetrieveCall<'_>,
    chunk: NonZeroUsize,
    events: &Events,
) -> Result<FileContentStreamingResult, ContentError>
where
    C: FileContentConfig,
    S: ClientSource,
{
    let prepared = fixture.prepare(deps, call, events)?;
    retrieve_content_streaming(prepared, chunk, &mut fixture.state).await
}

#[tokio::test]
async fn streaming_prepared_query_overrides_the_url_and_returns_before_the_body() {
    let events = Events::new();
    let drops = Arc::new(AtomicUsize::new(0));
    let (sender, body) = gated(&drops, 4);
    let wire = Wire::answering(
        streaming(200, &[("content-type", "application/octet-stream")], body),
        &events,
    );
    let source = Source::new(wire.clone());
    let config = recording_config(
        "http://mock.test/v1/files/f/content?x=old&y=2",
        QueryParams::from([
            (String::from("x"), String::from("new")),
            (String::from("p"), String::from("1")),
        ]),
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
    let call = retrieve_call(
        &content_request,
        &litellm,
        &headers,
        Some(Duration::from_secs(2)),
    );
    let mut result = fetch_stream(
        &mut fixture,
        &deps,
        &call,
        NonZeroUsize::new(3).expect("3 is not zero"),
        &events,
    )
    .await
    .expect("the stream opens before a byte exists");
    let exit = Arc::clone(&result.exit);
    let sent = wire.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, Method::GET);
    assert_eq!(
        sent[0].url.as_str(),
        "http://mock.test/v1/files/f/content?x=new&y=2&p=1"
    );
    assert!(sent[0].query.is_empty());
    assert_eq!(sent[0].timeout, Some(Duration::from_secs(2)));
    drop(sent);
    let streams = locked(&config.streams);
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
    drop(streams);
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
    let mut chunks = Vec::new();
    while let Some(chunk) = result.stream.next().await {
        chunks.push(chunk.expect("the body is read"));
    }
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
    let serving = serve(
        reply(
            "200 OK",
            &[("content-type", "application/octet-stream")],
            "late",
        ),
        Duration::from_millis(500),
    )
    .expect("the fixture serves");
    let events = Events::new();
    let config = recording_config(
        &format!("http://{}/v1/files/f/content", serving.address),
        QueryParams::new(),
        provider_headers(),
        &events,
    );
    let source = ProxiedSource::new(IMPATIENT);
    let deps = ContentDeps {
        config: &config,
        source: &source,
    };
    let litellm = params("http://unused.test");
    let headers = Headers::new();
    let content_request = request("f", IndexMap::new());
    let mut fixture = Fixture::new("file_content");
    let call = retrieve_call(&content_request, &litellm, &headers, None);
    let Err(error) = fetch_stream(
        &mut fixture,
        &deps,
        &call,
        NonZeroUsize::new(3).expect("3 is not zero"),
        &events,
    )
    .await
    else {
        panic!("the client read timeout fires");
    };
    let ContentError::Provider(failure) = error else {
        panic!("a timeout is a provider error");
    };
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
    let serving = serve(
        reply(
            "200 OK",
            &[("content-type", "application/octet-stream")],
            "late",
        ),
        Duration::from_millis(500),
    )
    .expect("the fixture serves");
    let events = Events::new();
    let config = recording_config(
        &format!("http://{}/v1/files/f/content", serving.address),
        QueryParams::new(),
        provider_headers(),
        &events,
    );
    let source = ProxiedSource::new(TransportLimits::DEFAULT);
    let deps = ContentDeps {
        config: &config,
        source: &source,
    };
    let litellm = params("http://unused.test");
    let headers = Headers::new();
    let content_request = request("f", IndexMap::new());
    let mut fixture = Fixture::new("file_content");
    let call = retrieve_call(
        &content_request,
        &litellm,
        &headers,
        Some(Duration::from_millis(100)),
    );
    let Err(error) = fetch_stream(
        &mut fixture,
        &deps,
        &call,
        NonZeroUsize::new(3).expect("3 is not zero"),
        &events,
    )
    .await
    else {
        panic!("the call timeout fires");
    };
    let ContentError::Provider(failure) = error else {
        panic!("a timeout is a provider error");
    };
    assert_eq!(failure.status_code, 500);
    assert!(failure.status_code_is_synthesized);
    assert!(failure.message.contains("total deadline"));
    let sent = serving.finish().expect("the fixture answers");
    assert_eq!(sent.line, "GET /v1/files/f/content HTTP/1.1");
}

#[tokio::test]
async fn streaming_answer_within_the_client_timeout_is_read() {
    let serving = serve(
        reply(
            "200 OK",
            &[("content-type", "application/octet-stream")],
            "late",
        ),
        Duration::from_millis(100),
    )
    .expect("the fixture serves");
    let events = Events::new();
    let config = recording_config(
        &format!("http://{}/v1/files/f/content", serving.address),
        QueryParams::new(),
        provider_headers(),
        &events,
    );
    let source = ProxiedSource::new(TransportLimits::DEFAULT);
    let deps = ContentDeps {
        config: &config,
        source: &source,
    };
    let litellm = params("http://unused.test");
    let headers = Headers::new();
    let content_request = request("f", IndexMap::new());
    let mut fixture = Fixture::new("file_content");
    let call = retrieve_call(&content_request, &litellm, &headers, None);
    let mut result = fetch_stream(
        &mut fixture,
        &deps,
        &call,
        NonZeroUsize::new(3).expect("3 is not zero"),
        &events,
    )
    .await
    .expect("the stream opens");
    let mut chunks = Vec::new();
    while let Some(chunk) = result.stream.next().await {
        chunks.push(chunk.expect("the body is read"));
    }
    assert_eq!(chunks, vec![b"lat".to_vec(), b"e".to_vec()]);
    assert_eq!(result.exit.get(), Some(&StreamExit::Completed));
    serving.finish().expect("the fixture answers");
}

#[tokio::test]
async fn streaming_error_body_is_lossy_and_closed_once() {
    let events = Events::new();
    let drops = Arc::new(AtomicUsize::new(0));
    let (sender, body) = gated(&drops, 4);
    sender
        .send(Ok(b"bad".to_vec()))
        .await
        .expect("the chunk is queued");
    sender
        .send(Ok(vec![0xFF]))
        .await
        .expect("the chunk is queued");
    drop(sender);
    let wire = Wire::answering(
        streaming(
            400,
            &[("content-type", "text/plain"), ("content-length", "4")],
            body,
        ),
        &events,
    );
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
    let Err(error) = fetch_stream(
        &mut fixture,
        &deps,
        &call,
        NonZeroUsize::new(3).expect("3 is not zero"),
        &events,
    )
    .await
    else {
        panic!("a 400 is a provider error");
    };
    let ContentError::Provider(failure) = error else {
        panic!("a 400 is a provider error");
    };
    assert_eq!(failure.status_code, 400);
    assert!(!failure.status_code_is_synthesized);
    assert_eq!(failure.message, "bad\u{fffd}");
    assert_eq!(
        failure.headers,
        ErrorHeaders::from([
            (
                String::from("content-length"),
                Json::Text(String::from("4")),
            ),
            (
                String::from("content-type"),
                Json::Text(String::from("text/plain")),
            ),
        ])
    );
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(locked(&config.streams).is_empty());
    assert_eq!(events.recorded(), vec!["log", "send"]);
    assert_eq!(
        config.errors(),
        vec![(
            String::from("bad\u{fffd}"),
            400_u16,
            Headers::from([
                (String::from("content-length"), String::from("4")),
                (String::from("content-type"), String::from("text/plain")),
            ]),
        )]
    );
}

#[tokio::test]
async fn streaming_error_body_reads_at_most_64_kib() {
    let events = Events::new();
    let drops = Arc::new(AtomicUsize::new(0));
    let (sender, body) = gated(&drops, 100);
    let queued = futures_util::future::join_all(
        (0..100_u16).map(|_index| sender.send(Ok(vec![b'a'; 1024]))),
    )
    .await;
    drop(sender);
    queued
        .into_iter()
        .collect::<Result<Vec<()>, _>>()
        .expect("every chunk is queued");
    let wire = Wire::answering(
        streaming(413, &[("content-length", "102400")], body),
        &events,
    );
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
    let Err(error) = fetch_stream(
        &mut fixture,
        &deps,
        &call,
        NonZeroUsize::new(3).expect("3 is not zero"),
        &events,
    )
    .await
    else {
        panic!("a 413 is a provider error");
    };
    let ContentError::Provider(failure) = error else {
        panic!("a 413 is a provider error");
    };
    assert_eq!(failure.status_code, 413);
    assert_eq!(failure.message.len(), 65_536);
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
    let result = fetch_stream(
        &mut fixture,
        &deps,
        &call,
        NonZeroUsize::new(2).expect("2 is not zero"),
        &events,
    )
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
            assert!(matches!(stream.next().await, None));
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
    let events = Events::new();
    let drops = Arc::new(AtomicUsize::new(0));
    let (sender, body) = gated(&drops, 1);
    let wire = Wire::answering(
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
        &events,
    );
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
    let result = fetch_stream(
        &mut fixture,
        &deps,
        &call,
        NonZeroUsize::new(3).expect("3 is not zero"),
        &events,
    )
    .await
    .expect("the stream opens");
    let streams = locked(&config.streams);
    assert_eq!(streams.len(), 1);
    assert!(matches!(streams[0].headers.get("content-encoding"), None));
    assert!(matches!(streams[0].headers.get("content-length"), None));
    assert_eq!(streams[0].headers.get_all("x-repeat").iter().count(), 2);
    assert_eq!(
        streams[0]
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/octet-stream")
    );
    assert_eq!(
        streams[0]
            .headers
            .get("content-disposition")
            .and_then(|value| value.to_str().ok()),
        Some("attachment; filename=\"a.bin\"")
    );
    drop(streams);
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
    let events = Events::new();
    let drops = Arc::new(AtomicUsize::new(0));
    let (sender, body) = gated(&drops, 1);
    let wire = Wire::answering(
        streaming(302, &[("location", "http://elsewhere.test/file")], body),
        &events,
    );
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
    let result = fetch_stream(
        &mut fixture,
        &deps,
        &call,
        NonZeroUsize::new(3).expect("3 is not zero"),
        &events,
    )
    .await
    .expect("a 3xx is an answer");
    assert_eq!(
        result.headers.get("location").map(String::as_str),
        Some("http://elsewhere.test/file")
    );
    assert_eq!(locked(&config.streams).len(), 1);
    assert!(config.errors().is_empty());
    drop(result);
    drop(sender);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
