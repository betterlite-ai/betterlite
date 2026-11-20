use std::time::Duration;

use betterlite_openai::error::ProviderError;
use tokio::io::AsyncReadExt as _;
use tokio::net::TcpListener;
use tokio::time::sleep;

use super::support::{
    BASE, Bench, ClientRequestRecord, Events, Fixture, IMPATIENT, ProxiedSource, RecordingConfig,
    Reply, TestError, Wire, content_call, generation_call, headers, params, video_deps,
};
use crate::api::videos_http_transport::client::{
    ClientError, ClientKind, ClientRequest, ClientSource,
};
use crate::api::videos_http_transport::error::{VideoTransportError, elapsed_text};
use crate::api::videos_http_transport::video_config::Params;

fn provider_error(error: VideoTransportError<TestError>) -> ProviderError {
    let VideoTransportError::Provider(provider) = error else {
        panic!("the error is not a provider error");
    };
    *provider
}

#[tokio::test]
async fn a_body_that_is_not_utf8_is_named_by_its_byte_count() {
    let mut bench = Bench::with_reply(Reply::bytes(400, Vec::from([0xff_u8, 0xfe])));
    let error = generation_error(&mut bench).await;
    assert_eq!(provider_error(error).message, "2 bytes that are not UTF-8");
}

#[tokio::test]
async fn a_header_value_that_is_not_utf8_is_named_by_its_byte_count() {
    let mut bench = Bench::with_reply(Reply::new(400, "x").with_raw_header("x-raw", &[0xff_u8]));
    let error = generation_error(&mut bench).await;
    let recorded = bench.config.errors();
    assert_eq!(
        recorded[0].2.get("x-raw").map(String::as_str),
        Some("1 bytes that are not UTF-8")
    );
    assert_eq!(provider_error(error).status_code, 400);
}

#[tokio::test]
async fn a_header_name_a_request_cannot_carry_is_a_synthesized_500() {
    let mut bench = Bench::new();
    let extra = headers(&[("bad name", "x")]);
    let optional = Params::new();
    let litellm = Params::new();
    let mut call = generation_call(&optional, &litellm);
    call.extra_headers = Some(&extra);
    let deps = video_deps(&bench.config, &bench.source, None);
    let error = bench
        .fixture
        .generate(&deps, &call, &bench.events, false)
        .await
        .expect_err("the header failure");
    let provider = provider_error(error);
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert!(provider.message.contains("header name"));
    assert!(bench.wire.requests().is_empty());
}

#[tokio::test]
async fn a_client_source_refusal_is_the_client_error() {
    let config = RecordingConfig::new(BASE);
    let source = Refusing;
    let deps = video_deps(&config, &source, None);
    let mut fixture = Fixture::new();
    let events = Events::new();
    let optional = Params::new();
    let litellm = Params::new();
    let call = generation_call(&optional, &litellm);
    let error = fixture
        .generate(&deps, &call, &events, false)
        .await
        .expect_err("the refusal");
    let VideoTransportError::Client(ClientError::UnknownProvider { provider }) = error else {
        panic!("the error is not a client error");
    };
    assert_eq!(provider, "nope");
}

struct Refusing;

impl ClientSource for Refusing {
    type Client = Wire;

    fn client(&self, _request: ClientRequest<'_>) -> Result<Wire, ClientError> {
        Err(ClientError::UnknownProvider {
            provider: "nope".to_owned(),
        })
    }
}

async fn generation_error(bench: &mut Bench) -> VideoTransportError<TestError> {
    let deps = video_deps(&bench.config, &bench.source, None);
    let optional = Params::new();
    let litellm = Params::new();
    let call = generation_call(&optional, &litellm);
    bench
        .fixture
        .generate(&deps, &call, &bench.events, false)
        .await
        .expect_err("the failing call")
}

#[test]
fn the_taken_time_keeps_the_python_float_shape() {
    assert_eq!(elapsed_text(Duration::from_secs(2)), "2.0");
    assert_eq!(elapsed_text(Duration::from_millis(1500)), "1.5");
    assert_eq!(elapsed_text(Duration::from_millis(1)), "0.001");
    assert_eq!(elapsed_text(Duration::ZERO), "0.0");
}

#[tokio::test(start_paused = true)]
async fn a_fractional_deadline_is_named_as_a_fraction() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener");
    let address = listener.local_addr().expect("the address");
    let config = RecordingConfig::new(&format!("http://{address}"));
    let source = ProxiedSource::new(IMPATIENT);
    let deps = video_deps(&config, &source, None);
    let optional = Params::new();
    let litellm = Params::new();
    let mut call = generation_call(&optional, &litellm);
    call.timeout = Duration::from_millis(1500);
    let events = Events::new();
    let mut fixture = Fixture::new();
    let (outcome, ()) = tokio::join!(
        fixture.generate(&deps, &call, &events, false),
        hold_open(listener)
    );
    let provider = provider_error(outcome.expect_err("the timeout"));
    assert_eq!(
        provider.message,
        "litellm.Timeout: Connection timed out after 1.5 seconds."
    );
}

#[tokio::test(start_paused = true)]
async fn a_content_post_without_a_timeout_names_none() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener");
    let address = listener.local_addr().expect("the address");
    let mut config = RecordingConfig::new(&format!("http://{address}"));
    config.content_data = params(r#"{"operation":"fetch"}"#);
    let source = ProxiedSource::new(IMPATIENT);
    let deps = video_deps(&config, &source, None);
    let litellm = Params::new();
    let call = content_call("video-1", &litellm);
    let mut fixture = Fixture::new();
    let (outcome, ()) = tokio::join!(fixture.content(&deps, &call, false), hold_open(listener));
    let provider = provider_error(outcome.expect_err("the timeout"));
    assert_eq!(provider.status_code, 408);
    assert_eq!(
        provider.message,
        "litellm.Timeout: Connection timed out after None seconds."
    );
}

#[tokio::test]
async fn the_client_source_records_the_kind_the_verify_and_the_provider() {
    let mut bench = Bench::new();
    let litellm = params(r#"{"ssl_verify":true}"#);
    let deps = video_deps(&bench.config, &bench.source, None);
    let optional = Params::new();
    let call = generation_call(&optional, &litellm);
    bench
        .fixture
        .generate(&deps, &call, &bench.events, true)
        .await
        .expect("the video");
    assert_eq!(
        bench.source.calls(),
        vec![ClientRequestRecord {
            kind: ClientKind::Asynchronous,
            ssl_verify: Some(true),
            provider: "openai".to_owned(),
        }]
    );
}

async fn hold_open(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.expect("the connection");
    let mut chunk = [0_u8; 4096];
    let read = stream.read(&mut chunk).await.expect("the request bytes");
    assert_ne!(read, 0, "the client sends its request before the deadline");
    sleep(Duration::from_secs(30)).await;
}
