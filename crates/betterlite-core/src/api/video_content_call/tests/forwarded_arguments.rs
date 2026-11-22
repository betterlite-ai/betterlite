use betterlite_models::decisions::Json;

use super::support::{
    Behavior, Bench, FakeProviders, Fixture, Journal, Outcome, REQUEST_TIMEOUT, VIDEO_ID,
    WIRE_BODY, Wire, call_with, handled, headers, kwargs, pending_bytes, ready, redactor, request,
    text,
};
use crate::api::video_content_call::VideoContentCaller;
use crate::api::video_content_call::handler::ContentTransportHandler;
use crate::api::videos_http_transport::client::{ClientKind, SuppliedClient};

#[test]
fn forwarded_arguments_content_reach_the_handler() {
    let wire = Wire::new();
    let mut bench = Bench::new(Outcome::Bytes(b"video".to_vec()));
    let mut request = request(VIDEO_ID);
    request.variant = Some("thumbnail".to_owned());
    request.extra_headers = Some(headers(&[("x-extra", "one")]));
    request.extra_query = Some(kwargs(&[("poll", text("1"))]));
    request.extra_body = Some(kwargs(&[("extra", text("body"))]));
    ready(call_with(
        &mut bench,
        request,
        SuppliedClient {
            kind: ClientKind::Sync,
            transport: &wire,
        },
    ));
    let handling = handled(&bench.journal);
    assert_eq!(handling.variant.as_deref(), Some("thumbnail"));
    assert_eq!(handling.extra_headers, Some(headers(&[("x-extra", "one")])));
    assert_eq!(handling.client, Some(ClientKind::Sync));
    assert!(wire.seen().is_empty());
}

#[test]
fn forwarded_arguments_content_transport_reaches_the_provider() {
    let journal = Journal::new();
    let caller = VideoContentCaller {
        providers: FakeProviders::new(&journal, Behavior::Found),
        handler: ContentTransportHandler,
        redactor: redactor(),
        request_timeout: REQUEST_TIMEOUT,
    };
    let mut fixture = Fixture::new();
    let wire = Wire::new();
    let mut request = request(VIDEO_ID);
    request.variant = Some("thumbnail".to_owned());
    request.extra_headers = Some(headers(&[("x-extra", "one")]));
    request.extra_query = Some(kwargs(&[("poll", text("1"))]));
    request.extra_body = Some(kwargs(&[("extra", text("body"))]));
    let work = caller.video_content(
        request,
        Some(SuppliedClient {
            kind: ClientKind::Sync,
            transport: &wire,
        }),
        &mut fixture.state,
    );
    assert_eq!(ready(work), WIRE_BODY.as_bytes().to_vec());
    let seen = wire.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].url, "http://mock.test/videos/video-1");
    let extra = ("x-extra".to_owned(), "one".to_owned());
    assert!(seen[0].headers.contains(&extra));
    let provider_header = ("x-provider".to_owned(), "gemini".to_owned());
    assert!(seen[0].headers.contains(&provider_header));
    let contexts = caller.providers.config.contexts();
    assert_eq!(contexts.len(), 1);
    assert_eq!(contexts[0].1.as_deref(), Some("thumbnail"));
    assert_eq!(
        contexts[0].2.get("x-extra").map(String::as_str),
        Some("one")
    );
    let environments = caller.providers.config.environments();
    assert_eq!(environments.len(), 1);
    assert_eq!(environments[0].model, "");
    assert_eq!(environments[0].api_key, None);
    assert_eq!(
        environments[0].headers.get("x-extra").map(String::as_str),
        Some("one")
    );
    assert!(caller.providers.source.calls().is_empty());
}

#[tokio::test]
async fn forwarded_arguments_content_transport_uses_the_async_client() {
    let journal = Journal::new();
    let caller = VideoContentCaller {
        providers: FakeProviders::new(&journal, Behavior::Found),
        handler: ContentTransportHandler,
        redactor: redactor(),
        request_timeout: REQUEST_TIMEOUT,
    };
    let mut fixture = Fixture::new();
    let wire = Wire::new();
    let mut request = request(VIDEO_ID);
    request.kwargs = kwargs(&[("async_call", Json::Bool(true))]);
    let work = caller.video_content(
        request,
        Some(SuppliedClient {
            kind: ClientKind::Asynchronous,
            transport: &wire,
        }),
        &mut fixture.state,
    );
    assert_eq!(pending_bytes(work).await, WIRE_BODY.as_bytes().to_vec());
    assert_eq!(wire.seen().len(), 1);
    assert!(caller.providers.source.calls().is_empty());
}
