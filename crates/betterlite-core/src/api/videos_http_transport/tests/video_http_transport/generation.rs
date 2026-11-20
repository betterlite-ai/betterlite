use std::time::Duration;

use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::decisions::Json;
use betterlite_openai::error::ProviderError;
use http::Method;
use rstest::rstest;
use tokio::io::AsyncReadExt as _;
use tokio::net::TcpListener;
use tokio::time::sleep;

use super::support::{
    BASE, Bench, ClientRequestRecord, CreateReply, Environment, Events, Failure, Fixture,
    IMPATIENT, MODEL, PROMPT, PROVIDER, ProxiedSource, RecordingConfig, Reply, Source, TestError,
    Wire, body_json, body_parts, generation_call, headers, params, request_file, secret,
    text_parts, video_deps,
};
use crate::api::video_records::VideoObject;
use crate::api::videos_http_transport::client::{ClientKind, SuppliedClient};
use crate::api::videos_http_transport::error::VideoTransportError;
use crate::api::videos_http_transport::video_config::Params;

async fn run(
    bench: &mut Bench,
    optional: &Params,
    litellm: &Params,
    is_async: bool,
) -> Result<VideoObject, VideoTransportError<TestError>> {
    let deps = video_deps(&bench.config, &bench.source, None);
    let call = generation_call(optional, litellm);
    bench
        .fixture
        .generate(&deps, &call, &bench.events, is_async)
        .await
}

fn provider_error(error: VideoTransportError<TestError>) -> ProviderError {
    let VideoTransportError::Provider(provider) = error else {
        panic!("the error is not a provider error");
    };
    *provider
}

#[tokio::test]
async fn a_request_with_files_sends_the_data_fields_before_the_file_parts() {
    let mut bench = Bench::new();
    bench.config.create_data = params(r#"{"prompt":"a cat","seconds":"4"}"#);
    bench.config.create_files = vec![request_file("input_reference", Some("clip.mp4"), None)];
    let video = run(&mut bench, &Params::new(), &Params::new(), false)
        .await
        .expect("the video");
    assert_eq!(
        video,
        VideoObject::new("video-1".to_owned(), "queued".to_owned())
    );
    let requests = bench.wire.requests();
    let request = &requests[0];
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.url.as_str(), "http://mock.test/v1/submit");
    assert_eq!(request.timeout, Some(Duration::from_secs(30)));
    let (boundary, parts) = body_parts(request);
    assert_eq!(boundary.len(), 64);
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0].name, "prompt");
    assert_eq!(parts[0].data.as_slice(), b"a cat");
    assert_eq!(parts[1].name, "seconds");
    assert_eq!(parts[2].name, "input_reference");
    assert_eq!(parts[2].file_name.as_deref(), Some("clip.mp4"));
    assert_eq!(parts[2].content_type.as_deref(), Some("video/mp4"));
    assert_eq!(parts[2].data.as_slice(), b"file-bytes");
}

#[tokio::test]
async fn the_multipart_flag_sends_a_fileless_form() {
    let mut bench = Bench::new();
    bench.config.multipart = true;
    bench.config.create_data.clone_from(&params(
        r#"{"prompt":"a cat","metadata":{"a":1},"tags":["x","y"]}"#,
    ));
    run(&mut bench, &Params::new(), &Params::new(), false)
        .await
        .expect("the video");
    let requests = bench.wire.requests();
    let (_, parts) = body_parts(&requests[0]);
    assert!(parts.iter().all(|part| part.file_name.is_none()));
    assert!(parts.iter().all(|part| part.content_type.is_none()));
    assert_eq!(
        text_parts(&requests[0]),
        vec![
            ("prompt".to_owned(), "a cat".to_owned()),
            ("metadata[a]".to_owned(), "1".to_owned()),
            ("tags[]".to_owned(), "x".to_owned()),
            ("tags[]".to_owned(), "y".to_owned()),
        ]
    );
}

#[tokio::test]
async fn files_go_out_even_when_the_multipart_flag_is_set() {
    let mut bench = Bench::new();
    bench.config.multipart = true;
    bench.config.create_data = params(r#"{"prompt":"a cat"}"#);
    bench.config.create_files = vec![request_file("input_reference", None, Some("image/png"))];
    run(&mut bench, &Params::new(), &Params::new(), false)
        .await
        .expect("the video");
    let requests = bench.wire.requests();
    let (_, parts) = body_parts(&requests[0]);
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[1].name, "input_reference");
    assert_eq!(parts[1].file_name, None);
    assert_eq!(parts[1].content_type.as_deref(), Some("image/png"));
}

#[tokio::test]
async fn without_files_or_the_flag_the_data_goes_out_as_json() {
    let mut bench = Bench::new();
    bench.config.create_data = params(r#"{"prompt":"a cat","seconds":"4"}"#);
    run(&mut bench, &Params::new(), &Params::new(), false)
        .await
        .expect("the video");
    let requests = bench.wire.requests();
    assert_eq!(
        body_json(&requests[0]),
        Json::Object(params(r#"{"prompt":"a cat","seconds":"4"}"#))
    );
}

#[tokio::test]
async fn the_request_goes_to_the_url_the_transform_returned() {
    let mut bench = Bench::new();
    bench.config.base = "http://other.test/base".to_owned();
    run(&mut bench, &Params::new(), &Params::new(), false)
        .await
        .expect("the video");
    let requests = bench.wire.requests();
    assert_eq!(requests[0].url.as_str(), "http://other.test/base/submit");
    assert_eq!(
        bench.config.create_contexts(),
        vec![(
            "http://other.test/base".to_owned(),
            headers(&[
                ("Authorization", "Bearer sk-provider"),
                ("Content-Type", "application/json"),
            ]),
        )]
    );
}

#[tokio::test]
async fn the_headers_take_the_optional_params_then_the_environment_then_the_call() {
    let mut bench = Bench::new();
    let optional =
        params(r#"{"extra_headers":{"X-Optional":"opt","Content-Type":"application/optional"}}"#);
    let litellm = Params::new();
    let mut call = generation_call(&optional, &litellm);
    let extra = headers(&[("Content-Type", "application/call"), ("X-Call", "call")]);
    call.extra_headers = Some(&extra);
    let deps = video_deps(&bench.config, &bench.source, None);
    bench
        .fixture
        .generate(&deps, &call, &bench.events, false)
        .await
        .expect("the video");
    let environments = bench.config.environments();
    assert_eq!(
        environments[0].headers,
        headers(&[
            ("X-Optional", "opt"),
            ("Content-Type", "application/optional")
        ])
    );
    let requests = bench.wire.requests();
    let seen = &requests[0].headers;
    assert_eq!(
        seen.get("authorization").unwrap().to_str().unwrap(),
        "Bearer sk-provider"
    );
    assert_eq!(seen.get("x-optional").unwrap(), "opt");
    assert_eq!(seen.get("x-call").unwrap(), "call");
    assert_eq!(seen.get("content-type").unwrap(), "application/call");
}

#[tokio::test]
async fn the_pre_call_record_carries_the_prompt_base_headers_and_body() {
    let mut bench = Bench::new();
    bench.config.create_data = params(r#"{"prompt":"a cat"}"#);
    run(&mut bench, &Params::new(), &Params::new(), false)
        .await
        .expect("the video");
    assert_eq!(bench.events.recorded(), ["log", "send"]);
    assert_eq!(bench.fixture.input(), Some(&Json::Text(PROMPT.to_owned())));
    assert_eq!(
        bench.fixture.logged("api_base"),
        Some(&Json::Text(format!("{BASE}/submit")))
    );
    assert_eq!(
        bench.fixture.logged("complete_input_dict"),
        Some(&Json::Object(params(r#"{"prompt":"a cat"}"#)))
    );
    let Some(Json::Object(logged)) = bench.fixture.logged("headers") else {
        panic!("the headers are logged");
    };
    assert_eq!(
        logged.get("Authorization"),
        Some(&Json::Text("Be****er".to_owned()))
    );
    assert_eq!(
        logged.get("Content-Type"),
        Some(&Json::Text("application/json".to_owned()))
    );
}

#[tokio::test]
async fn a_failing_status_becomes_the_provider_error_with_its_headers() {
    let mut bench = Bench::with_reply(
        Reply::new(429, r#"{"error":"slow down"}"#)
            .with_header("x-request-id", "req-1")
            .with_header("content-encoding", "gzip")
            .with_header("content-length", "20"),
    );
    let error = run(&mut bench, &Params::new(), &Params::new(), false)
        .await
        .expect_err("the failing status");
    let provider = provider_error(error);
    assert_eq!(provider.status_code, 429);
    assert!(!provider.status_code_is_synthesized);
    assert_eq!(provider.message, r#"{"error":"slow down"}"#);
    let recorded = bench.config.errors();
    assert_eq!(recorded[0].0, r#"{"error":"slow down"}"#);
    assert_eq!(recorded[0].1, 429);
    assert_eq!(
        recorded[0].2.get("x-request-id").map(String::as_str),
        Some("req-1")
    );
    assert!(!recorded[0].2.contains_key("content-encoding"));
    assert!(!recorded[0].2.contains_key("content-length"));
}

#[rstest]
#[case::created(201_u16)]
#[case::just_below_client_error(399_u16)]
#[tokio::test]
async fn a_successful_status_goes_to_the_transform_unchanged(#[case] status: u16) {
    let mut bench = Bench::with_reply(Reply::new(status, "{}"));
    run(&mut bench, &Params::new(), &Params::new(), false)
        .await
        .expect("the video");
    assert!(bench.config.errors().is_empty());
    assert_eq!(
        bench.config.transforms(),
        ["create_request", "create_response"]
    );
}

#[rstest]
#[case::bad_request(400_u16)]
#[case::server_error(500_u16)]
#[tokio::test]
async fn four_hundred_and_up_is_a_provider_error(#[case] status: u16) {
    let mut bench = Bench::with_reply(Reply::new(status, "boom"));
    let error = run(&mut bench, &Params::new(), &Params::new(), false)
        .await
        .expect_err("the failing status");
    assert_eq!(provider_error(error).status_code, status);
}

#[tokio::test]
async fn the_create_response_transform_sees_the_model_provider_and_body() {
    let mut bench = Bench::new();
    bench.config.create_data = params(r#"{"prompt":"a cat"}"#);
    run(&mut bench, &Params::new(), &Params::new(), true)
        .await
        .expect("the video");
    assert_eq!(
        bench.config.create_replies(),
        vec![CreateReply {
            model: MODEL.to_owned(),
            provider: Some(PROVIDER.to_owned()),
            request_data: Some(params(r#"{"prompt":"a cat"}"#)),
        }]
    );
    assert_eq!(
        bench.fixture.state.model_call_details.get("response_mark"),
        Some(&Json::Text("created".to_owned()))
    );
}

#[tokio::test]
async fn the_create_response_transform_failure_is_not_a_provider_error() {
    let mut bench = Bench::new();
    bench.config.failures.push(Failure::CreateResponse);
    let error = run(&mut bench, &Params::new(), &Params::new(), false)
        .await
        .expect_err("the transform failure");
    assert!(matches!(
        error,
        VideoTransportError::Config(TestError::Transform)
    ));
    assert!(bench.config.errors().is_empty());
    assert_eq!(
        bench
            .fixture
            .state
            .model_call_details
            .get("create_failed_mark"),
        Some(&Json::Bool(true)),
        "the failing transform wrote to the live logging object"
    );
}

#[tokio::test]
async fn the_create_request_transform_failure_is_not_a_provider_error() {
    let mut bench = Bench::new();
    bench.config.failures.push(Failure::CreateRequest);
    let error = run(&mut bench, &Params::new(), &Params::new(), false)
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
    let optional = Params::new();
    let litellm = Params::new();
    let call = generation_call(&optional, &litellm);
    let error = fixture
        .generate(&deps, &call, &events, false)
        .await
        .expect_err("the transport failure");
    let provider = provider_error(error);
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert_eq!(
        provider.message,
        "the client is shutting down and takes no new requests"
    );
}

#[tokio::test]
async fn a_url_that_cannot_be_parsed_is_a_synthesized_500() {
    let mut bench = Bench::new();
    bench.config.base = "not a url".to_owned();
    let error = run(&mut bench, &Params::new(), &Params::new(), false)
        .await
        .expect_err("the url failure");
    let provider = provider_error(error);
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert!(provider.message.contains("is not a URL"));
    assert!(bench.wire.requests().is_empty());
}

#[tokio::test]
async fn a_header_a_request_cannot_carry_is_a_synthesized_500() {
    let mut bench = Bench::new();
    let extra = headers(&[("X-Bad", "line\nbreak")]);
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
    assert!(provider.message.contains("header value"));
}

#[rstest]
#[case::sync(false, ClientKind::Sync)]
#[case::asynchronous(true, ClientKind::Asynchronous)]
#[tokio::test]
async fn the_client_source_gets_the_kind_the_ssl_verify_and_the_provider(
    #[case] is_async: bool,
    #[case] kind: ClientKind,
) {
    let mut bench = Bench::new();
    let litellm = params(r#"{"ssl_verify":false}"#);
    run(&mut bench, &Params::new(), &litellm, is_async)
        .await
        .expect("the video");
    assert_eq!(
        bench.source.calls(),
        vec![ClientRequestRecord {
            kind,
            ssl_verify: Some(false),
            provider: PROVIDER.to_owned(),
        }]
    );
}

#[tokio::test]
async fn a_supplied_client_of_the_matching_kind_is_used() {
    let mut bench = Bench::new();
    let supplied = SuppliedClient {
        kind: ClientKind::Sync,
        transport: &bench.wire,
    };
    let deps = video_deps(&bench.config, &bench.source, Some(supplied));
    let optional = Params::new();
    let litellm = Params::new();
    let call = generation_call(&optional, &litellm);
    bench
        .fixture
        .generate(&deps, &call, &bench.events, false)
        .await
        .expect("the video");
    assert!(bench.source.calls().is_empty());
    assert_eq!(bench.wire.requests().len(), 1);
}

#[tokio::test]
async fn a_supplied_client_of_the_other_kind_is_not_used() {
    let mut bench = Bench::new();
    let supplied_wire = Wire::replying(Reply::new(200, "video-bytes"), &bench.events);
    let supplied = SuppliedClient {
        kind: ClientKind::Sync,
        transport: &supplied_wire,
    };
    let deps = video_deps(&bench.config, &bench.source, Some(supplied));
    let optional = Params::new();
    let litellm = Params::new();
    let call = generation_call(&optional, &litellm);
    bench
        .fixture
        .generate(&deps, &call, &bench.events, true)
        .await
        .expect("the video");
    assert_eq!(bench.source.calls().len(), 1);
    assert_eq!(bench.source.calls()[0].kind, ClientKind::Asynchronous);
    assert!(supplied_wire.requests().is_empty());
    assert_eq!(bench.wire.requests().len(), 1);
}

#[tokio::test]
async fn the_environment_gets_the_model_and_the_litellm_params() {
    let mut bench = Bench::new();
    let litellm = params(r#"{"api_base":"http://conf.test/v2"}"#);
    run(&mut bench, &Params::new(), &litellm, false)
        .await
        .expect("the video");
    assert_eq!(
        bench.config.environments(),
        vec![Environment {
            headers: headers(&[]),
            model: MODEL.to_owned(),
            api_key: None,
            litellm_params: Some(litellm.clone()),
        }]
    );
    assert_eq!(
        bench.config.urls(),
        vec![(MODEL.to_owned(), Some("http://conf.test/v2".to_owned()))]
    );
}

#[tokio::test]
async fn the_api_key_argument_wins_over_the_litellm_params_key() {
    let mut bench = Bench::new();
    let litellm = params(r#"{"api_key":"sk-params"}"#);
    let optional = Params::new();
    let key = secret("sk-call");
    let mut call = generation_call(&optional, &litellm);
    call.api_key = Some(&key);
    let deps = video_deps(&bench.config, &bench.source, None);
    bench
        .fixture
        .generate(&deps, &call, &bench.events, false)
        .await
        .expect("the video");
    assert_eq!(
        bench.config.environments()[0].api_key.as_deref(),
        Some("sk-call")
    );
}

#[tokio::test]
async fn the_litellm_params_key_is_used_when_no_argument_is_given() {
    let mut bench = Bench::new();
    let litellm = params(r#"{"api_key":"sk-params"}"#);
    run(&mut bench, &Params::new(), &litellm, false)
        .await
        .expect("the video");
    assert_eq!(
        bench.config.environments()[0].api_key.as_deref(),
        Some("sk-params")
    );
}

#[tokio::test]
async fn a_bad_api_base_is_a_field_error() {
    let mut bench = Bench::new();
    let litellm = params(r#"{"api_base":7}"#);
    let error = run(&mut bench, &Params::new(), &litellm, false)
        .await
        .expect_err("the field error");
    let VideoTransportError::Field(field) = error else {
        panic!("the error is not a field error");
    };
    assert_eq!(field.field, "litellm_params.api_base");
    assert_eq!(field.expected, "a string");
}

#[tokio::test]
async fn a_bad_ssl_verify_is_a_field_error() {
    let mut bench = Bench::new();
    let litellm = params(r#"{"ssl_verify":"yes"}"#);
    let error = run(&mut bench, &Params::new(), &litellm, false)
        .await
        .expect_err("the field error");
    let VideoTransportError::Field(field) = error else {
        panic!("the error is not a field error");
    };
    assert_eq!(field.field, "litellm_params.ssl_verify");
    assert_eq!(field.expected, "a boolean");
}

#[tokio::test]
async fn a_bad_optional_extra_headers_value_is_a_field_error() {
    let mut bench = Bench::new();
    let optional = params(r#"{"extra_headers":{"X-Key":7}}"#);
    let error = run(&mut bench, &optional, &Params::new(), false)
        .await
        .expect_err("the field error");
    let VideoTransportError::Field(field) = error else {
        panic!("the error is not a field error");
    };
    assert_eq!(
        field.field,
        "video_generation_optional_request_params.extra_headers"
    );
    assert_eq!(field.expected, "a map of string to string");
}

#[tokio::test]
async fn a_generation_reaches_the_provider_over_the_wire() {
    let serving = serve(reply("200 OK", &[], r#"{"id":"video-1"}"#), Duration::ZERO)
        .expect("the fixture serves");
    let base = format!("http://{}/v1", serving.address);
    let mut config = RecordingConfig::new(&base);
    config.create_data = params(r#"{"prompt":"a cat"}"#);
    let source = ProxiedSource::new(TransportLimits::DEFAULT);
    let deps = video_deps(&config, &source, None);
    let optional = Params::new();
    let litellm = Params::new();
    let call = generation_call(&optional, &litellm);
    let events = Events::new();
    let mut fixture = Fixture::new();
    let video = fixture
        .generate(&deps, &call, &events, false)
        .await
        .expect("the video");
    let request = serving.finish().expect("the recorded request");
    assert_eq!(video.id, "video-1");
    assert_eq!(request.line, "POST /v1/submit HTTP/1.1");
    let head = request.head.to_lowercase();
    assert!(head.contains("authorization: bearer sk-provider"));
    assert!(head.contains("content-type: application/json"));
    assert_eq!(request.body, r#"{"prompt":"a cat"}"#);
}

#[tokio::test(start_paused = true)]
async fn a_sync_timeout_is_the_litellm_timeout_with_status_408() {
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
    call.timeout = Duration::from_secs(2);
    let events = Events::new();
    let mut fixture = Fixture::new();
    let (outcome, ()) = tokio::join!(
        fixture.generate(&deps, &call, &events, false),
        hold_open(listener)
    );
    let provider = provider_error(outcome.expect_err("the timeout"));
    assert_eq!(provider.status_code, 408);
    assert!(!provider.status_code_is_synthesized);
    assert_eq!(
        provider.message,
        "litellm.Timeout: Connection timed out after 2 seconds."
    );
}

#[tokio::test(start_paused = true)]
async fn an_async_timeout_names_the_timeout_and_the_time_taken() {
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
    call.timeout = Duration::from_secs(2);
    let events = Events::new();
    let mut fixture = Fixture::new();
    let (outcome, ()) = tokio::join!(
        fixture.generate(&deps, &call, &events, true),
        hold_open(listener)
    );
    let provider = provider_error(outcome.expect_err("the timeout"));
    assert_eq!(provider.status_code, 408);
    assert!(
        provider
            .message
            .starts_with("litellm.Timeout: Connection timed out. Timeout passed=2, time taken=")
    );
    assert!(provider.message.ends_with(" seconds"));
}

async fn hold_open(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.expect("the connection");
    let mut chunk = [0_u8; 4096];
    let read = stream.read(&mut chunk).await.expect("the request bytes");
    assert_ne!(read, 0, "the client sends its request before the deadline");
    sleep(Duration::from_secs(30)).await;
}
