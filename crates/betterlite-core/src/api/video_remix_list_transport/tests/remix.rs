use std::time::Duration;

use betterlite_http::proxy::limits::TransportLimits;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use http::Method;

use super::recording::RecordingConfig;
use super::support::{
    Bench, CallKind, CallMode, Events, Reply, Setup, Source, Wire, assert_validated, fake_bench,
    headers, json_params, json_server, proxied_bench, request_body, request_header, run, secret,
    video_object,
};
use crate::api::video_remix_list_transport::client::VideoClient;
use crate::api::video_remix_list_transport::error::VideoRemixListTransportError;
use crate::api::video_remix_list_transport::records::{Headers, Params};

const REMIX: &str = "video_remix";
const PROMPT: &str = "make it slow";
const EVENTS: [&str; 6] = [
    "validate",
    "url",
    "remix_request",
    "log",
    "send",
    "remix_response",
];

#[derive(Debug, Clone, Copy)]
pub(super) enum RemixCase {
    SyncSuppliedClient,
    SyncPooledClient,
    AsyncPooledClient,
    AsyncSuppliedClient,
    AsyncRealServer,
    ApiKeyArgument,
    PreCall,
    IsAsyncHandOff,
    ResponseUnchanged,
}

pub(super) async fn remix_case(case: RemixCase) {
    match case {
        RemixCase::SyncSuppliedClient => sync_supplied_client().await,
        RemixCase::SyncPooledClient => sync_pooled_client().await,
        RemixCase::AsyncPooledClient => async_pooled_client().await,
        RemixCase::AsyncSuppliedClient => async_supplied_client().await,
        RemixCase::AsyncRealServer => async_real_server().await,
        RemixCase::ApiKeyArgument => api_key_argument().await,
        RemixCase::PreCall => pre_call().await,
        RemixCase::IsAsyncHandOff => is_async_hand_off().await,
        RemixCase::ResponseUnchanged => response_unchanged().await,
    }
}

// A supplied sync client is used as it is: the provider name is never parsed and the pool is never touched
async fn sync_supplied_client() {
    let (mut bench, pooled) = fake_bench(REMIX, Reply::json(200, "{}"), RecordingConfig::new);
    let supplied = Wire::replying(Reply::json(200, "{}"), &bench.events);
    let params = json_params(
        r#"{"api_base":"https://caller.test/v1","api_key":"sk-params","ssl_verify":true}"#,
    );
    let extra_headers = headers(&[("X-Trace", "trace-a")]);
    let extra_body = json_params(r#"{"prompt":"zoom in"}"#);
    let argument = secret("sk-argument");
    let mut setup = Setup::new("nope", &params).video("video_a", PROMPT);
    setup.client = Some(VideoClient::Sync(&supplied));
    setup.extra_headers = Some(&extra_headers);
    setup.extra_body = Some(&extra_body);
    setup.timeout = Some(Duration::from_secs(2));
    setup.api_key = Some(&argument);
    let answer = run(&mut bench, setup, CallKind::Remix, CallMode::Sync)
        .await
        .expect("the sync remix handler answers");
    assert_eq!(answer.object(), video_object("video_out", "queued"));
    assert!(pooled.requests().is_empty());
    assert!(bench.transport.source.sync_calls().is_empty());
    assert!(bench.transport.source.async_calls().is_empty());
    assert_sync_supplied(&bench, &supplied, &params, &extra_body);
}

fn assert_sync_supplied(
    bench: &Bench<Source>,
    supplied: &Wire,
    params: &Params,
    extra_body: &Params,
) {
    assert_sync_supplied_records(bench, params, extra_body);
    let sent = supplied.requests();
    assert_eq!(sent.len(), 1);
    let request = &sent[0];
    assert_eq!(request.method, Method::POST);
    assert_eq!(
        request.url.as_str(),
        "https://api.test/v1/videos/video_a/remix"
    );
    assert!(request.query.is_empty());
    assert_eq!(request.timeout, Some(Duration::from_secs(2)));
    assert_eq!(
        request_header(request, "authorization"),
        "Bearer sk-provider"
    );
    assert_eq!(request_header(request, "content-type"), "application/json");
    // the caller's header survives the config's own X-Trace because the extra headers are re-applied
    assert_eq!(request_header(request, "x-trace"), "trace-a");
    assert_eq!(
        request_body(request),
        Json::Object(bench.config.remix_body().clone())
    );
}

fn assert_sync_supplied_records(bench: &Bench<Source>, params: &Params, extra_body: &Params) {
    assert_validated(
        bench,
        Some("sk-argument"),
        Some("https://caller.test/v1"),
        &headers(&[("X-Trace", "trace-a")]),
        params,
    );
    assert_remix_inputs(bench, params, Some(extra_body));
    assert_eq!(bench.events.recorded(), EVENTS);
    let replies = bench.config.replies();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].status, 200);
    assert_eq!(replies[0].provider.as_deref(), Some("nope"));
    assert_eq!(
        bench
            .fixture
            .state
            .model_call_details
            .get("video_transform"),
        Some(&Json::Text(String::from("remix")))
    );
}

fn assert_remix_inputs(bench: &Bench<Source>, params: &Params, extra_body: Option<&Params>) {
    let inputs = bench.config.remix_inputs();
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].video_id, "video_a");
    assert_eq!(inputs[0].prompt, PROMPT);
    assert_eq!(inputs[0].extra_body.as_ref(), extra_body);
    assert_eq!(inputs[0].api_base, bench.config.base());
    assert_eq!(inputs[0].litellm_params, *params);
    assert_eq!(
        inputs[0].headers.get("X-Trace").map(String::as_str),
        Some("trace-a")
    );
    assert_eq!(
        inputs[0].headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-provider")
    );
}

// A supplied async client is the wrong mode for the sync path, so the pooled sync client sends, chosen by ssl_verify alone
async fn sync_pooled_client() {
    let (mut bench, pooled) = fake_bench(REMIX, Reply::json(200, "{}"), RecordingConfig::new);
    let supplied = Wire::replying(Reply::json(200, "{}"), &bench.events);
    let params = json_params(r#"{"api_key":"sk-params","ssl_verify":true}"#);
    let mut setup = Setup::new("openai", &params).video("video_a", PROMPT);
    setup.client = Some(VideoClient::Asynchronous(&supplied));
    let answer = run(&mut bench, setup, CallKind::Remix, CallMode::Sync)
        .await
        .expect("the sync remix handler answers");
    assert_eq!(answer.object(), video_object("video_out", "queued"));
    assert_eq!(bench.transport.source.sync_calls(), vec![Some(true)]);
    assert!(bench.transport.source.async_calls().is_empty());
    assert!(supplied.requests().is_empty());
    let sent = pooled.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].timeout, None);
    // with no caller headers the config's own X-Trace is the one that travels
    assert_eq!(request_header(&sent[0], "x-trace"), "config-trace");
    assert_validated(&bench, Some("sk-params"), None, &headers(&[]), &params);
}

// The async remix path answers with the transform's object and posts once to the remix URL, whichever client sent
async fn async_remix_post(bench: &mut Bench<Source>, setup: Setup<'_, Source>, sent: &Wire) {
    let answer = run(bench, setup, CallKind::Remix, CallMode::Asynchronous)
        .await
        .expect("the async remix handler answers");
    assert_eq!(answer.object(), video_object("video_out", "queued"));
    assert!(bench.transport.source.sync_calls().is_empty());
    let requests = sent.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::POST);
    assert_eq!(requests[0].url.as_str(), bench.config.remix_url());
}

// The async path takes a supplied async client as it is: the unknown provider name is never parsed and the pool is never touched
async fn async_supplied_client() {
    let (mut bench, pooled) = fake_bench(REMIX, Reply::json(200, "{}"), RecordingConfig::new);
    let supplied = Wire::replying(Reply::json(200, "{}"), &bench.events);
    let params = json_params("{}");
    let mut setup = Setup::new("nope", &params).video("video_a", PROMPT);
    setup.client = Some(VideoClient::Asynchronous(&supplied));
    async_remix_post(&mut bench, setup, &supplied).await;
    assert!(pooled.requests().is_empty());
    assert!(bench.transport.source.async_calls().is_empty());
}

// The async path keys its pool by the provider name and passes ssl_verify to it
async fn async_pooled_client() {
    let (mut bench, wire) = fake_bench(REMIX, Reply::json(200, "{}"), RecordingConfig::new);
    let params = json_params(r#"{"ssl_verify":false}"#);
    let mut setup = Setup::new("openai", &params).video("video_a", PROMPT);
    setup.timeout = Some(Duration::from_secs(3));
    async_remix_post(&mut bench, setup, &wire).await;
    assert_eq!(
        bench.transport.source.async_calls(),
        vec![(ProviderId::Openai, Some(false))]
    );
    assert_eq!(wire.requests()[0].timeout, Some(Duration::from_secs(3)));
}

// The pooled client is the real B173 transport, and its POST carries the JSON body and the config's headers
async fn async_real_server() {
    const VIDEO: &str = r#"{"id":"video_out","status":"queued"}"#;
    let serving = json_server(VIDEO, Duration::ZERO);
    let address = serving.address;
    let configure = move |events: &Events| {
        RecordingConfig::new(events)
            .with_base(&format!("http://{address}/v1"))
            .with_remix_url(&format!("http://{address}/v1/videos/video_a/remix"))
    };
    let mut bench = proxied_bench(REMIX, configure, TransportLimits::DEFAULT);
    let params = json_params(r#"{"api_key":"sk-params"}"#);
    let mut setup = Setup::new("openai", &params).video("video_a", PROMPT);
    setup.timeout = Some(Duration::from_secs(10));
    let answer = run(&mut bench, setup, CallKind::Remix, CallMode::Asynchronous)
        .await
        .expect("the async remix handler answers");
    assert_eq!(answer.object(), video_object("video_out", "queued"));
    let sent = serving.finish().expect("the fixture answers");
    assert!(
        sent.line
            .starts_with("POST /v1/videos/video_a/remix HTTP/1.1")
    );
    assert!(sent.head.contains("authorization: Bearer sk-provider"));
    assert!(sent.head.contains("content-type: application/json"));
    assert_eq!(
        sent.body,
        serde_json::to_string(bench.config.remix_body()).expect("the body encodes")
    );
    let replies = bench.config.replies();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].status, 200);
    assert_eq!(replies[0].body, VIDEO.as_bytes().to_vec());
}

// The api_key argument wins over a params api_key that is not even a string, so the params value is never parsed
async fn api_key_argument() {
    let (mut bench, _) = fake_bench(REMIX, Reply::json(200, "{}"), RecordingConfig::new);
    let params = json_params(r#"{"api_key":7}"#);
    let argument = secret("sk-argument");
    let mut setup = Setup::new("openai", &params).video("video_a", PROMPT);
    setup.api_key = Some(&argument);
    let answer = run(&mut bench, setup, CallKind::Remix, CallMode::Asynchronous)
        .await
        .expect("the async remix handler answers");
    assert_eq!(answer.object(), video_object("video_out", "queued"));
    assert_eq!(
        bench.config.api_keys(),
        vec![Some(String::from("sk-argument"))]
    );
}

// pre_call sees the prompt, an api-key-free record and the body, the base URL, the headers and the video id
async fn pre_call() {
    let (mut bench, _) = fake_bench(REMIX, Reply::json(200, "{}"), RecordingConfig::new);
    let params = json_params("{}");
    let extra_headers = headers(&[("X-Trace", "trace-a")]);
    let mut setup = Setup::new("openai", &params).video("video_a", PROMPT);
    setup.extra_headers = Some(&extra_headers);
    let answer = run(&mut bench, setup, CallKind::Remix, CallMode::Asynchronous)
        .await
        .expect("the async remix handler answers");
    assert_eq!(answer.object(), video_object("video_out", "queued"));
    assert_remix_pre_call(&bench, &extra_headers);
}

fn assert_remix_pre_call(bench: &Bench<Source>, extra_headers: &Headers) {
    assert_eq!(
        bench.fixture.input(),
        Some(&Json::Text(String::from(PROMPT)))
    );
    assert_eq!(
        bench.fixture.logged("api_base"),
        Some(&Json::Text(bench.config.remix_url().to_owned()))
    );
    assert_eq!(
        bench.fixture.logged("complete_input_dict"),
        Some(&Json::Object(bench.config.remix_body().clone()))
    );
    assert_eq!(
        bench.fixture.logged("video_id"),
        Some(&Json::Text(String::from("video_a")))
    );
    assert!(bench.fixture.logged("params").is_none());
    let Some(Json::Object(logged)) = bench.fixture.logged("headers") else {
        panic!("the pre-call logs the headers as an object");
    };
    assert_eq!(
        logged.get("X-Trace"),
        Some(&Json::Text(String::from("trace-a")))
    );
    assert_eq!(
        logged.get("Content-Type"),
        Some(&Json::Text(String::from("application/json")))
    );
    assert_eq!(
        logged.get("Authorization"),
        Some(&Json::Text(String::from("Be****er")))
    );
    assert_eq!(logged.len(), extra_headers.len().saturating_add(2));
    assert_eq!(bench.events.recorded(), EVENTS);
}

// _is_async hands the async handler's future back before any client is picked, even for an unknown provider
async fn is_async_hand_off() {
    let (mut bench, _) = fake_bench(REMIX, Reply::json(200, "{}"), RecordingConfig::new);
    let params = json_params("{}");
    let setup = Setup::new("openai", &params).video("video_a", PROMPT);
    let answer = run(&mut bench, setup, CallKind::Remix, CallMode::HandOff)
        .await
        .expect("the hand-off runs the async handler");
    assert_eq!(answer.object(), video_object("video_out", "queued"));
    assert_eq!(
        bench.transport.source.async_calls(),
        vec![(ProviderId::Openai, None)]
    );
    assert!(bench.transport.source.sync_calls().is_empty());
    assert_unknown_provider_hand_off(&params).await;
}

async fn assert_unknown_provider_hand_off(params: &Params) {
    let (mut bench, wire) = fake_bench(REMIX, Reply::json(200, "{}"), RecordingConfig::new);
    let setup = Setup::new("nope", params).video("video_a", PROMPT);
    let error = run(&mut bench, setup, CallKind::Remix, CallMode::HandOff)
        .await
        .expect_err("an unknown provider fails inside the future, not at the hand-off");
    assert!(matches!(
        error,
        VideoRemixListTransportError::UnknownProvider { .. }
    ));
    assert!(bench.transport.source.sync_calls().is_empty());
    assert!(bench.transport.source.async_calls().is_empty());
    assert!(wire.requests().is_empty());
    assert!(bench.config.events().is_empty());
}

// The response transform's object is returned as it is, hidden params included
async fn response_unchanged() {
    let mut object = video_object("video_out", "in_progress");
    object.progress = Some(42);
    object.model = Some(String::from("sora-2"));
    object
        .hidden_params
        .insert(String::from("provider"), Json::Text(String::from("openai")));
    let configured = object.clone();
    let (mut bench, _) = fake_bench(REMIX, Reply::json(200, "{}"), move |events| {
        RecordingConfig::new(events).with_remix_object(configured)
    });
    let params = json_params("{}");
    let setup = Setup::new("openai", &params).video("video_a", PROMPT);
    let answer = run(&mut bench, setup, CallKind::Remix, CallMode::Asynchronous)
        .await
        .expect("the async remix handler answers");
    assert_eq!(answer.object(), object);
}
