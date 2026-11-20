use std::time::Duration;

use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::decisions::Json;
use betterlite_openai::files::records::BinaryContent;
use http::Method;
use indexmap::IndexMap;
use rstest::rstest;

use super::support::{
    Answer, Events, Fixture, ProxiedSource, Source, Wire, buffered, caller_headers, extra_body,
    key, locked, params, provider_headers, recording_config, request, retrieve_call,
    unavailable_config, wire_header,
};
use crate::api::file_http_content::config::{
    ContentParams, FileContentConfig, Headers, QueryParams,
};
use crate::api::file_http_content::error::ContentError;
use crate::api::file_http_content::service::buffered::retrieve_file_content;
use crate::api::file_http_content::service::{ClientSource, ContentDeps, RetrieveCall};
use crate::api::vector_store_http_write::error::HeaderMap as ErrorHeaders;

const BINARY: [u8; 10] = [0x00, 0xFF, 0x62, 0x69, 0x6E, 0x61, 0x72, 0x79, 0x80, 0x0A];

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
