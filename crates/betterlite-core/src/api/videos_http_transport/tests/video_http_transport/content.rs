use std::time::Duration;

use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use http::Method;
use tokio::io::AsyncReadExt as _;
use tokio::net::TcpListener;

use super::support::{
    BASE, Bench, ClientRequestRecord, Events, Failure, Fixture, IMPATIENT, PROVIDER, ProxiedSource,
    RecordingConfig, Reply, Source, TestError, Wire, body_json, content_call, headers, params,
    secret, video_deps,
};
use crate::api::videos_http_transport::client::{ClientKind, SuppliedClient};
use crate::api::videos_http_transport::error::VideoTransportError;
use crate::api::videos_http_transport::video_config::Params;

async fn run(
    bench: &mut Bench,
    video_id: &str,
    litellm: &Params,
    is_async: bool,
) -> Result<Vec<u8>, VideoTransportError<TestError>> {
    let deps = video_deps(&bench.config, &bench.source, None);
    let call = content_call(video_id, litellm);
    bench.fixture.content(&deps, &call, is_async).await
}

fn provider_status(error: VideoTransportError<TestError>) -> u16 {
    let VideoTransportError::Provider(provider) = error else {
        panic!("the error is not a provider error");
    };
    provider.status_code
}

#[tokio::test]
async fn params_from_the_transform_go_out_as_a_post_without_a_timeout() {
    let mut bench = Bench::new();
    bench.config.content_data = params(r#"{"operation":"fetchPredictOperation"}"#);
    let bytes = run(&mut bench, "video-1", &Params::new(), false)
        .await
        .expect("the content");
    assert_eq!(bytes.as_slice(), "video-bytes".as_bytes());
    let requests = bench.wire.requests();
    let request = &requests[0];
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.url.as_str(), "http://mock.test/v1/contents/video-1");
    assert_eq!(request.timeout, None);
    assert_eq!(
        body_json(request),
        Json::Object(params(r#"{"operation":"fetchPredictOperation"}"#))
    );
}

#[tokio::test]
async fn an_empty_params_map_sends_a_get_with_no_body() {
    let mut bench = Bench::new();
    let bytes = run(&mut bench, "video-1", &Params::new(), false)
        .await
        .expect("the content");
    assert_eq!(bytes.as_slice(), "video-bytes".as_bytes());
    let requests = bench.wire.requests();
    let request = &requests[0];
    assert_eq!(request.method, Method::GET);
    assert_eq!(request.url.as_str(), "http://mock.test/v1/contents/video-1");
    assert_eq!(request.url.query(), None);
    assert_eq!(request.timeout, None);
    assert!(matches!(request.body, RequestBody::Empty));
}

#[tokio::test]
async fn the_content_url_comes_from_the_transform_and_the_litellm_api_base() {
    let mut bench = Bench::new();
    let litellm = params(r#"{"api_base":"http://conf.test/v3"}"#);
    run(&mut bench, "video-9", &litellm, false)
        .await
        .expect("the content");
    assert_eq!(
        bench.config.urls(),
        vec![(String::new(), Some("http://conf.test/v3".to_owned()))]
    );
    let requests = bench.wire.requests();
    assert_eq!(
        requests[0].url.as_str(),
        "http://mock.test/v1/contents/video-9"
    );
    assert_eq!(
        bench.config.content_contexts(),
        vec![(BASE.to_owned(), None)]
    );
}

#[tokio::test]
async fn the_variant_reaches_the_content_transform() {
    let mut bench = Bench::new();
    let litellm = Params::new();
    let mut call = content_call("video-1", &litellm);
    call.variant = Some("thumbnail");
    let deps = video_deps(&bench.config, &bench.source, None);
    bench
        .fixture
        .content(&deps, &call, false)
        .await
        .expect("the content");
    assert_eq!(
        bench.config.content_contexts(),
        vec![(BASE.to_owned(), Some("thumbnail".to_owned()))]
    );
}

#[tokio::test]
async fn the_call_headers_start_the_environment_and_go_over_its_answer() {
    let mut bench = Bench::new();
    let extra = headers(&[("Content-Type", "application/call"), ("X-Call", "call")]);
    let litellm = Params::new();
    let mut call = content_call("video-1", &litellm);
    call.extra_headers = Some(&extra);
    let deps = video_deps(&bench.config, &bench.source, None);
    bench
        .fixture
        .content(&deps, &call, false)
        .await
        .expect("the content");
    assert_eq!(bench.config.environments()[0].headers, extra);
    let requests = bench.wire.requests();
    let seen = &requests[0].headers;
    assert_eq!(seen.get("authorization").unwrap(), "Bearer sk-provider");
    assert_eq!(seen.get("x-call").unwrap(), "call");
    assert_eq!(seen.get("content-type").unwrap(), "application/call");
}

#[tokio::test]
async fn the_environment_gets_an_empty_model_and_the_argument_key() {
    let mut bench = Bench::new();
    let litellm = params(r#"{"api_key":"sk-params"}"#);
    let key = secret("sk-call");
    let mut call = content_call("video-1", &litellm);
    call.api_key = Some(&key);
    let deps = video_deps(&bench.config, &bench.source, None);
    bench
        .fixture
        .content(&deps, &call, false)
        .await
        .expect("the content");
    let environment = bench.config.environments();
    assert_eq!(environment[0].model, "");
    assert_eq!(environment[0].api_key.as_deref(), Some("sk-call"));
}

#[tokio::test]
async fn the_litellm_params_key_is_not_a_content_key() {
    let mut bench = Bench::new();
    let litellm = params(r#"{"api_key":"sk-params"}"#);
    run(&mut bench, "video-1", &litellm, false)
        .await
        .expect("the content");
    assert_eq!(bench.config.environments()[0].api_key, None);
}

#[tokio::test]
async fn a_failing_content_post_status_is_a_provider_error() {
    let mut bench = Bench::with_reply(Reply::new(400, "bad request"));
    bench.config.content_data = params(r#"{"operation":"fetch"}"#);
    let error = run(&mut bench, "video-1", &Params::new(), false)
        .await
        .expect_err("the failing status");
    assert_eq!(provider_status(error), 400);
}

#[tokio::test]
async fn a_failing_get_status_reaches_the_content_transform() {
    let mut bench = Bench::with_reply(Reply::new(500, "boom"));
    let bytes = run(&mut bench, "video-1", &Params::new(), false)
        .await
        .expect("the content");
    assert_eq!(bytes.as_slice(), "boom".as_bytes());
    assert_eq!(
        bench.config.content_replies(),
        vec![(500_u16, "boom".as_bytes().to_vec())]
    );
    assert!(bench.config.errors().is_empty());
}

#[tokio::test]
async fn the_sync_content_transform_runs_for_a_sync_call() {
    let mut bench = Bench::new();
    run(&mut bench, "video-1", &Params::new(), false)
        .await
        .expect("the content");
    assert_eq!(
        bench.config.transforms(),
        ["content_request", "content_sync"]
    );
    assert_eq!(
        bench.fixture.state.model_call_details.get("content_mark"),
        Some(&Json::Text("sync".to_owned()))
    );
}

#[tokio::test]
async fn the_async_content_transform_runs_for_an_async_call() {
    let mut bench = Bench::new();
    run(&mut bench, "video-1", &Params::new(), true)
        .await
        .expect("the content");
    assert_eq!(
        bench.config.transforms(),
        ["content_request", "content_async"]
    );
    assert_eq!(
        bench.fixture.state.model_call_details.get("content_mark"),
        Some(&Json::Text("async".to_owned()))
    );
    assert_eq!(
        bench.source.calls(),
        vec![ClientRequestRecord {
            kind: ClientKind::Asynchronous,
            ssl_verify: None,
            provider: PROVIDER.to_owned(),
        }]
    );
}

#[tokio::test]
async fn a_sync_content_transform_failure_is_a_synthesized_provider_error() {
    let mut bench = Bench::new();
    bench.config.failures.push(Failure::ContentResponse);
    let error = run(&mut bench, "video-1", &Params::new(), false)
        .await
        .expect_err("the transform failure");
    let VideoTransportError::Provider(provider) = error else {
        panic!("the error is not a provider error");
    };
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert_eq!(provider.message, "the transform refused");
    let recorded = bench.config.errors();
    assert_eq!(recorded[0].1, 500);
    assert_eq!(
        recorded[0].2.get("x-transform").map(String::as_str),
        Some("raised")
    );
    assert_eq!(
        bench
            .fixture
            .state
            .model_call_details
            .get("content_failed_mark"),
        Some(&Json::Bool(true)),
        "the failing transform wrote to the live logging object"
    );
}

#[tokio::test]
async fn an_async_content_transform_failure_is_a_synthesized_provider_error() {
    let mut bench = Bench::new();
    bench.config.failures.push(Failure::ContentResponse);
    let error = run(&mut bench, "video-1", &Params::new(), true)
        .await
        .expect_err("the transform failure");
    assert_eq!(provider_status(error), 500);
    assert_eq!(
        bench.config.transforms(),
        ["content_request", "content_async"]
    );
    assert_eq!(
        bench
            .fixture
            .state
            .model_call_details
            .get("content_failed_mark"),
        Some(&Json::Bool(true)),
        "the failing transform wrote to the live logging object"
    );
}

#[tokio::test]
async fn a_content_transform_error_with_a_status_keeps_it() {
    let mut bench = Bench::new();
    bench.config.failures.push(Failure::ContentResponse);
    bench.config.content_error_status = Some(429);
    let error = run(&mut bench, "video-1", &Params::new(), false)
        .await
        .expect_err("the transform failure");
    let VideoTransportError::Provider(provider) = error else {
        panic!("the error is not a provider error");
    };
    assert_eq!(provider.status_code, 429);
    assert!(!provider.status_code_is_synthesized);
}

#[tokio::test]
async fn the_content_request_transform_failure_is_not_a_provider_error() {
    let mut bench = Bench::new();
    bench.config.failures.push(Failure::ContentRequest);
    let error = run(&mut bench, "video-1", &Params::new(), false)
        .await
        .expect_err("the transform failure");
    assert!(matches!(
        error,
        VideoTransportError::Config(TestError::Transform)
    ));
    assert!(bench.wire.requests().is_empty());
}

#[tokio::test]
async fn a_transport_failure_is_a_synthesized_500() {
    let events = Events::new();
    let source = Source::new(Wire::failing(TransportError::Closed, &events));
    let config = RecordingConfig::new(BASE);
    let deps = video_deps(&config, &source, None);
    let mut fixture = Fixture::new();
    let litellm = Params::new();
    let call = content_call("video-1", &litellm);
    let error = fixture
        .content(&deps, &call, false)
        .await
        .expect_err("the transport failure");
    let VideoTransportError::Provider(provider) = error else {
        panic!("the error is not a provider error");
    };
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
}

#[tokio::test]
async fn an_async_content_call_does_not_use_a_sync_supplied_client() {
    let mut bench = Bench::new();
    let supplied_wire = Wire::replying(Reply::new(200, "video-bytes"), &bench.events);
    let supplied = SuppliedClient {
        kind: ClientKind::Sync,
        transport: &supplied_wire,
    };
    let deps = video_deps(&bench.config, &bench.source, Some(supplied));
    let litellm = Params::new();
    let call = content_call("video-1", &litellm);
    bench
        .fixture
        .content(&deps, &call, true)
        .await
        .expect("the content");
    assert_eq!(bench.source.calls().len(), 1);
    assert_eq!(bench.source.calls()[0].kind, ClientKind::Asynchronous);
    assert!(supplied_wire.requests().is_empty());
    assert_eq!(bench.wire.requests().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_content_get_past_the_client_timeout_is_a_synthesized_500() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener");
    let address = listener.local_addr().expect("the address");
    let config = RecordingConfig::new(&format!("http://{address}"));
    let source = ProxiedSource::new(IMPATIENT);
    let deps = video_deps(&config, &source, None);
    let litellm = Params::new();
    let call = content_call("video-1", &litellm);
    let mut fixture = Fixture::new();
    let (outcome, ()) = tokio::join!(fixture.content(&deps, &call, false), hold_open(listener));
    let error = outcome.expect_err("the timeout");
    let VideoTransportError::Provider(provider) = error else {
        panic!("the error is not a provider error");
    };
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert_eq!(
        provider.message,
        "the request ran past its read timeout or total deadline"
    );
}

#[tokio::test]
async fn a_content_call_the_client_refuses_keeps_its_field_error() {
    let mut bench = Bench::new();
    let litellm = params(r#"{"ssl_verify":"no"}"#);
    let error = run(&mut bench, "video-1", &litellm, false)
        .await
        .expect_err("the field error");
    let VideoTransportError::Field(field) = error else {
        panic!("the error is not a field error");
    };
    assert_eq!(field.field, "litellm_params.ssl_verify");
}

async fn hold_open(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.expect("the connection");
    let mut chunk = [0_u8; 4096];
    let read = stream.read(&mut chunk).await.expect("the request bytes");
    assert_ne!(read, 0, "the client sends its request before the deadline");
    tokio::time::sleep(Duration::from_secs(30)).await;
}
