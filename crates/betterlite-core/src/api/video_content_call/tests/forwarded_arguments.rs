use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support::{
    Bench, FakeProviders, Fixture, Journal, Outcome, RecordingConfig, VIDEO_ID, WIRE_BODY, Wire,
    call_with, handled, headers, kwargs, pending_bytes, ready, request, text, transport_caller,
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

// the provider side of the transport call: its wire request and the transform inputs it was given
fn assert_provider_reached(
    caller: &VideoContentCaller<FakeProviders, ContentTransportHandler>,
    wire: &Wire,
) {
    let seen = wire.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].url, "http://mock.test/videos/video-1");
    let extra = ("x-extra".to_owned(), "one".to_owned());
    assert!(seen[0].headers.contains(&extra));
    let provider = ("x-provider".to_owned(), "gemini".to_owned());
    assert!(seen[0].headers.contains(&provider));
    let contexts = caller.providers.config.contexts();
    assert_eq!(contexts.len(), 1);
    let variant = contexts[0].1.as_deref();
    assert_eq!(variant, Some("thumbnail"));
    let context_header = contexts[0].2.get("x-extra").map(String::as_str);
    assert_eq!(context_header, Some("one"));
    let environments = caller.providers.config.environments();
    assert_eq!(environments.len(), 1);
    assert_eq!(environments[0].model, "");
    assert_eq!(environments[0].api_key, None);
    let params = environments[0].litellm_params.as_ref();
    assert!(params.is_some_and(IndexMap::is_empty));
    let environment_header = environments[0].headers.get("x-extra").map(String::as_str);
    assert_eq!(environment_header, Some("one"));
    assert!(caller.providers.source.calls().is_empty());
}

#[test]
fn forwarded_arguments_content_transport_reaches_the_provider() {
    let journal = Journal::new();
    let caller = transport_caller(&journal, RecordingConfig::new());
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
    assert_provider_reached(&caller, &wire);
}

// LiteLLM never hands the call an api_key, so a request's own api_key never reaches the provider config
#[test]
fn forwarded_arguments_content_never_hands_the_caller_api_key_to_the_config() {
    let journal = Journal::new();
    let caller = transport_caller(&journal, RecordingConfig::new());
    let mut fixture = Fixture::new();
    let wire = Wire::new();
    let mut request = request(VIDEO_ID);
    request.kwargs = kwargs(&[("api_key", text("sk-caller"))]);
    let work = caller.video_content(
        request,
        Some(SuppliedClient {
            kind: ClientKind::Sync,
            transport: &wire,
        }),
        &mut fixture.state,
    );
    assert_eq!(ready(work), WIRE_BODY.as_bytes().to_vec());
    let environments = caller.providers.config.environments();
    assert_eq!(environments.len(), 1);
    assert_eq!(environments[0].api_key, None);
}

#[tokio::test]
async fn forwarded_arguments_content_transport_uses_the_async_client() {
    let journal = Journal::new();
    let caller = transport_caller(&journal, RecordingConfig::new());
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
