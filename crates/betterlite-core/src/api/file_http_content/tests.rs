#![cfg(test)]

mod support;

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::decisions::Json;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::records::{BinaryContent, FileContentRequest};
use betterlite_openai::files::stream::{FileContentStreamingResult, StreamExit};
use futures_util::StreamExt as _;
use http::Method;
use http::header::HeaderValue;
use indexmap::IndexMap;
use rstest::rstest;

use self::support::{
    Answer, Events, Fixture, IMPATIENT, ProxiedSource, Source, Wire, buffered, caller_headers,
    extra_body, gated, http_headers, key, params, provider_headers, recording_config, request,
    streaming, unavailable_config,
};
use crate::api::file_http_content::config::{
    ContentParams, FileContentConfig, Headers, QueryParams,
};
use crate::api::file_http_content::error::ContentError;
use crate::api::file_http_content::service::{
    ClientSource, ContentDeps, ContentRequest, RetrieveCall, decoded_body_headers,
    retrieve_file_content, retrieve_file_content_streaming,
};
use crate::api::vector_store_http_write::error::HeaderMap as ErrorHeaders;

const BINARY: [u8; 10] = [0x00, 0xFF, 0x62, 0x69, 0x6E, 0x61, 0x72, 0x79, 0x80, 0x0A];

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn retrieve_call<'call>(
    request: &'call FileContentRequest,
    params: &'call ContentParams,
    headers: &'call Headers,
    timeout: Option<Duration>,
) -> RetrieveCall<'call> {
    RetrieveCall {
        request,
        params,
        headers,
        timeout,
    }
}

fn wire_header<'request>(request: &'request ContentRequest, name: &str) -> Option<&'request str> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
}

async fn fetch<'deps, C, S>(
    fixture: &mut Fixture,
    deps: &'deps ContentDeps<'deps, C, S>,
    call: &RetrieveCall<'_>,
    events: &Events,
) -> Result<BinaryContent, ContentError>
where
    C: FileContentConfig,
    S: ClientSource,
{
    let prepared = fixture.prepare(deps, call, events)?;
    retrieve_file_content(prepared, &mut fixture.state).await
}

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
    retrieve_file_content_streaming(prepared, chunk, &mut fixture.state).await
}

#[tokio::test]
async fn preparation_logs_the_transformed_request() {
    let events = Events::new();
    let wire = Wire::answering(buffered(200, &[], &[0]), &events);
    let source = Source::new(wire.clone());
    let config = recording_config(
        "http://mock.test/v1/files/f/content",
        QueryParams::from([(String::from("p"), String::from("1"))]),
        provider_headers(),
        &events,
    );
    let deps = ContentDeps {
        config: &config,
        source: &source,
    };
    let litellm = ContentParams {
        api_base: Some(String::from("http://mock.test/v1")),
        api_key: Some(key("sk-explicit")),
    };
    let headers = caller_headers(&[("x-trace", "trace-a")]);
    let content_request = request("f", extra_body(&[("ignored", "yes")]));
    let mut fixture = Fixture::new("file_content");
    let call = retrieve_call(
        &content_request,
        &litellm,
        &headers,
        Some(Duration::from_secs(2)),
    );
    let content = fetch(&mut fixture, &deps, &call, &events)
        .await
        .expect("the content arrives");
    assert_eq!(content.status, 200);
    assert_eq!(
        content.headers.get("x-transform").map(String::as_str),
        Some("recorded")
    );
    let transforms = locked(&config.requests);
    assert_eq!(transforms.len(), 1);
    assert_eq!(transforms[0].file_id, "f");
    assert_eq!(transforms[0].extra_body, extra_body(&[("ignored", "yes")]));
    assert_eq!(
        transforms[0].api_base.as_deref(),
        Some("http://mock.test/v1")
    );
    drop(transforms);
    assert_eq!(*locked(&config.caller_headers), vec![headers.clone()]);
    assert_eq!(
        *locked(&config.api_keys),
        vec![Some(String::from("sk-explicit"))]
    );
    let sent = wire.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, Method::GET);
    assert_eq!(sent[0].url.as_str(), "http://mock.test/v1/files/f/content");
    assert_eq!(sent[0].query, vec![(String::from("p"), String::from("1"))]);
    assert_eq!(sent[0].timeout, None);
    assert_eq!(wire_header(&sent[0], "api_key"), Some("sk-provider"));
    assert_eq!(wire_header(&sent[0], "x-params-key"), Some("sk-explicit"));
    assert_eq!(wire_header(&sent[0], "x-trace"), Some("trace-a"));
    assert_eq!(
        wire_header(&sent[0], "content-type"),
        Some("application/json")
    );
    drop(sent);
    assert_eq!(fixture.input(), Some(&Json::Text(String::new())));
    assert_eq!(
        fixture.logged("file_id"),
        Some(&Json::Text(String::from("f")))
    );
    assert_eq!(
        fixture.logged("api_base"),
        Some(&Json::Text(String::from(
            "http://mock.test/v1/files/f/content"
        )))
    );
    let logged = fixture
        .logged("headers")
        .expect("the pre-call logs the validated headers");
    let Json::Object(logged) = logged else {
        panic!("the logged headers are an object");
    };
    assert_eq!(
        logged.get("api_key"),
        Some(&Json::Text(String::from("sk****er")))
    );
    assert_eq!(
        logged.get("x-params-key"),
        Some(&Json::Text(String::from("sk****it")))
    );
    assert_eq!(
        logged.get("x-trace"),
        Some(&Json::Text(String::from("tr****-a")))
    );
    assert_eq!(
        logged.get("content-type"),
        Some(&Json::Text(String::from("application/json")))
    );
    assert_eq!(events.recorded(), vec!["log", "send", "transform"]);
}

#[tokio::test]
async fn binary_body_reaches_the_response_transform_unchanged() {
    let events = Events::new();
    let wire = Wire::answering(
        buffered(
            200,
            &[("content-type", "application/octet-stream")],
            &BINARY,
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
    let content = fetch(&mut fixture, &deps, &call, &events)
        .await
        .expect("the content arrives");
    let responses = locked(&config.responses);
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].status_code, 200);
    assert_eq!(responses[0].body, BINARY);
    assert_eq!(responses[0].url, "http://mock.test/v1/files/f/content");
    assert_eq!(
        responses[0].headers.get("content-type").map(String::as_str),
        Some("application/octet-stream")
    );
    assert_eq!(content.body, BINARY.to_vec());
}

#[derive(Debug, Clone, Copy)]
enum SetupCase {
    Transform,
    Url,
    HeaderName,
    HeaderValue,
}

#[rstest]
#[case::transform(SetupCase::Transform)]
#[case::url(SetupCase::Url)]
#[case::header_name(SetupCase::HeaderName)]
#[case::header_value(SetupCase::HeaderValue)]
#[tokio::test]
async fn setup_failure_sends_nothing(#[case] case: SetupCase) {
    let events = Events::new();
    let wire = Wire::answering(buffered(200, &[], b"unused"), &events);
    let source = Source::new(wire.clone());
    let config = match case {
        SetupCase::Transform => unavailable_config(&events),
        SetupCase::Url => {
            recording_config("not a url", QueryParams::new(), provider_headers(), &events)
        }
        SetupCase::HeaderName => recording_config(
            "http://mock.test/files",
            QueryParams::new(),
            Headers::from([(String::from("bad name"), String::from("x"))]),
            &events,
        ),
        SetupCase::HeaderValue => recording_config(
            "http://mock.test/files",
            QueryParams::new(),
            Headers::from([(String::from("x-bad"), String::from("line\nbreak"))]),
            &events,
        ),
    };
    let deps = ContentDeps {
        config: &config,
        source: &source,
    };
    let litellm = params("http://unused.test");
    let headers = Headers::new();
    let content_request = request("f", IndexMap::new());
    let mut fixture = Fixture::new("file_content");
    let call = retrieve_call(&content_request, &litellm, &headers, None);
    let Err(error) = fetch(&mut fixture, &deps, &call, &events).await else {
        panic!("the setup fails");
    };
    match case {
        SetupCase::Transform => assert!(matches!(error, ContentError::Transform(_))),
        SetupCase::Url => {
            assert!(matches!(&error, ContentError::RequestUrl { url, .. } if url == "not a url"))
        }
        SetupCase::HeaderName => {
            assert!(matches!(&error, ContentError::HeaderName { name, .. } if name == "bad name"))
        }
        SetupCase::HeaderValue => {
            assert!(matches!(&error, ContentError::HeaderValue { name, .. } if name == "x-bad"))
        }
    }
    assert!(wire.requests().is_empty());
    assert!(events.recorded().is_empty());
}

#[tokio::test]
async fn status_error_uses_the_body_text_and_raw_headers() {
    let events = Events::new();
    let wire = Wire::answering(
        buffered(
            404,
            &[
                ("content-type", "text/plain"),
                ("retry-after", "3"),
                ("content-length", "7"),
            ],
            b"missing",
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
    let Err(error) = fetch(&mut fixture, &deps, &call, &events).await else {
        panic!("a 404 is a provider error");
    };
    let ContentError::Provider(failure) = error else {
        panic!("a 404 is a provider error");
    };
    assert_eq!(failure.status_code, 404);
    assert!(!failure.status_code_is_synthesized);
    assert_eq!(failure.message, "missing");
    assert_eq!(
        failure.headers,
        ErrorHeaders::from([
            (
                String::from("content-length"),
                Json::Text(String::from("7")),
            ),
            (
                String::from("content-type"),
                Json::Text(String::from("text/plain")),
            ),
            (String::from("retry-after"), Json::Text(String::from("3"))),
        ])
    );
    assert!(locked(&config.responses).is_empty());
    assert_eq!(wire.requests().len(), 1);
    assert_eq!(events.recorded(), vec!["log", "send"]);
    assert_eq!(
        config.errors(),
        vec![(
            String::from("missing"),
            404_u16,
            Headers::from([
                (String::from("content-length"), String::from("7")),
                (String::from("content-type"), String::from("text/plain")),
                (String::from("retry-after"), String::from("3")),
            ]),
        )]
    );
}

#[tokio::test]
async fn invalid_utf8_status_body_reads_as_replacement_characters() {
    let events = Events::new();
    let wire = Wire::answering(buffered(500, &[], b"bad\xff"), &events);
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
    let Err(error) = fetch(&mut fixture, &deps, &call, &events).await else {
        panic!("a 500 is a provider error");
    };
    let ContentError::Provider(failure) = error else {
        panic!("a 500 is a provider error");
    };
    assert_eq!(failure.status_code, 500);
    assert!(!failure.status_code_is_synthesized);
    assert_eq!(failure.message, "bad\u{fffd}");
    assert!(failure.headers.is_empty());
}

#[tokio::test]
async fn transport_failure_is_a_synthesized_500() {
    let events = Events::new();
    let wire = Wire::answering(Answer::Failure(TransportError::Closed), &events);
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
    let Err(error) = fetch(&mut fixture, &deps, &call, &events).await else {
        panic!("a transport failure is a provider error");
    };
    let ContentError::Provider(failure) = error else {
        panic!("a transport failure is a provider error");
    };
    assert_eq!(failure.status_code, 500);
    assert!(failure.status_code_is_synthesized);
    assert_eq!(failure.message, TransportError::Closed.to_string());
    assert!(failure.headers.is_empty());
    assert!(locked(&config.responses).is_empty());
    assert_eq!(events.recorded(), vec!["log", "send"]);
}

#[tokio::test]
async fn non_error_status_reaches_the_response_transform() {
    let events = Events::new();
    let wire = Wire::answering(
        buffered(
            302,
            &[("location", "http://elsewhere.test/file")],
            b"redirect",
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
    let content = fetch(&mut fixture, &deps, &call, &events)
        .await
        .expect("a 3xx is an answer");
    assert_eq!(content.status, 302);
    assert_eq!(content.body, b"redirect".to_vec());
    assert_eq!(locked(&config.responses).len(), 1);
    assert!(config.errors().is_empty());
    assert_eq!(events.recorded(), vec!["log", "send", "transform"]);
}

#[tokio::test]
async fn buffered_url_query_wins_over_the_transformed_parameters() {
    let serving = serve(
        reply(
            "200 OK",
            &[("content-type", "application/octet-stream")],
            "bytes",
        ),
        Duration::ZERO,
    )
    .expect("the fixture serves");
    let events = Events::new();
    let config = recording_config(
        &format!("http://{}/v1/files/f/content?x=old", serving.address),
        QueryParams::from([
            (String::from("x"), String::from("new")),
            (String::from("p"), String::from("1")),
        ]),
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
    let content = fetch(&mut fixture, &deps, &call, &events)
        .await
        .expect("the content arrives");
    assert_eq!(content.body, b"bytes".to_vec());
    let sent = serving.finish().expect("the fixture answers");
    assert_eq!(sent.line, "GET /v1/files/f/content?x=old&p=1 HTTP/1.1");
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
    for _index in 0..100_u16 {
        sender
            .send(Ok(vec![b'a'; 1024]))
            .await
            .expect("the chunk is queued");
    }
    drop(sender);
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

#[rstest]
#[case::absent(None)]
#[case::identity(Some("identity"))]
#[case::upper(Some("IDENTITY"))]
#[case::mixed(Some("Identity"))]
fn decoded_headers_keep_an_identity_encoding(#[case] encoding: Option<&str>) {
    let mut headers = http_headers(&[
        ("content-type", "application/octet-stream"),
        ("content-disposition", "attachment; filename=\"a.bin\""),
        ("x-repeat", "one"),
    ]);
    headers.append("x-repeat", HeaderValue::from_static("two"));
    if let Some(value) = encoding {
        headers.insert(
            "content-encoding",
            HeaderValue::from_str(value).expect("the header value"),
        );
    }
    assert_eq!(decoded_body_headers(&headers), headers);
}

#[rstest]
#[case::gzip(&[("content-encoding", "gzip")])]
#[case::empty(&[("content-encoding", "")])]
#[case::repeated(&[
    ("content-encoding", "identity"),
    ("content-encoding", "identity"),
])]
fn decoded_headers_drop_stale_transfer_headers(#[case] stale: &[(&'static str, &'static str)]) {
    let mut pairs: Vec<(&'static str, &'static str)> = stale.to_vec();
    pairs.push(("content-length", "10"));
    pairs.push(("content-disposition", "attachment"));
    pairs.push(("content-type", "application/octet-stream"));
    pairs.push(("x-repeat", "one"));
    let mut headers = http_headers(&pairs);
    headers.append("content-length", HeaderValue::from_static("11"));
    headers.append("x-repeat", HeaderValue::from_static("two"));
    let decoded = decoded_body_headers(&headers);
    assert!(matches!(decoded.get("content-encoding"), None));
    assert!(matches!(decoded.get("content-length"), None));
    assert_eq!(
        decoded
            .get("content-disposition")
            .and_then(|value| value.to_str().ok()),
        Some("attachment")
    );
    assert_eq!(
        decoded
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/octet-stream")
    );
    let repeats: Vec<&str> = decoded
        .get_all("x-repeat")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect();
    assert_eq!(repeats, vec!["one", "two"]);
    assert_eq!(headers.get_all("content-length").iter().count(), 2);
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
