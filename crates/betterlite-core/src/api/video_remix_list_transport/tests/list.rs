use std::time::Duration;

use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use http::Method;
use tokio::runtime::{Builder, Runtime};

use super::recording::RecordingConfig;
use super::support::{
    Answer, Bench, CallKind, CallMode, Events, LIST_PAGE, Reply, Setup, Source, Wire,
    assert_validated, fake_bench, headers, json_params, proxied_bench, run, run_sync_list, secret,
};
use crate::api::video_remix_list_transport::client::VideoClient;
use crate::api::video_remix_list_transport::error::VideoRemixListTransportError;
use crate::api::video_remix_list_transport::records::Params;

const LIST: &str = "video_list";
const EVENTS: [&str; 6] = [
    "validate",
    "url",
    "list_request",
    "log",
    "send",
    "list_response",
];

#[derive(Debug, Clone, Copy)]
pub(super) enum ListCase {
    ApiKeyIgnoredFromParams,
    ApiKeyArgument,
    GetQueryAndNoTimeout,
    AsyncSuppliedClient,
    RealGet,
    SyncRunsToCompletion,
    HandOffForwardsApiKey,
    RunningRuntime,
}

pub(super) fn list_case(case: ListCase) {
    match case {
        ListCase::ApiKeyIgnoredFromParams => test_runtime().block_on(api_key_ignored_from_params()),
        ListCase::ApiKeyArgument => test_runtime().block_on(api_key_argument()),
        ListCase::GetQueryAndNoTimeout => test_runtime().block_on(get_query_and_no_timeout()),
        ListCase::AsyncSuppliedClient => test_runtime().block_on(async_supplied_client()),
        ListCase::RealGet => test_runtime().block_on(real_get()),
        ListCase::SyncRunsToCompletion => sync_runs_to_completion(),
        ListCase::HandOffForwardsApiKey => test_runtime().block_on(hand_off_forwards_api_key()),
        ListCase::RunningRuntime => running_runtime(),
    }
}

fn test_runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime builds")
}

// validate_environment gets the api_key argument, which is none here, so the params' api_key is never read
async fn api_key_ignored_from_params() {
    let (mut bench, _) = fake_bench(LIST, Reply::json(200, "{}"), RecordingConfig::new);
    let params = json_params(r#"{"api_key":"sk-params"}"#);
    let setup = Setup::new("openai", &params);
    let answer = run(&mut bench, setup, CallKind::List, CallMode::Asynchronous)
        .await
        .expect("the list handler answers");
    assert_eq!(answer.page(), bench.config.list_page().clone());
    assert_validated(&bench, None, None, &headers(&[]));
}

// The api_key argument is what validate_environment gets, even with a params api_key set
async fn api_key_argument() {
    let (mut bench, _) = fake_bench(LIST, Reply::json(200, "{}"), RecordingConfig::new);
    let params = json_params(r#"{"api_key":7}"#);
    let argument = secret("sk-argument");
    let mut setup = Setup::new("openai", &params);
    setup.api_key = Some(&argument);
    let answer = run(&mut bench, setup, CallKind::List, CallMode::Asynchronous).await;
    assert!(matches!(answer, Ok(Answer::Page(_))));
    assert_validated(&bench, Some("sk-argument"), None, &headers(&[]));
}

// The GET carries the transformed params as its query and no timeout, and the pre-call logs the empty input and the params
async fn get_query_and_no_timeout() {
    let extra_query = json_params(r#"{"include":"x"}"#);
    let configure = |events: &Events| {
        RecordingConfig::new(events).with_list_params(json_params(
            r#"{"after":"video_a","limit":2,"order":"desc","tag":["a","b"]}"#,
        ))
    };
    let (mut bench, wire) = fake_bench(LIST, Reply::json(200, "{}"), configure);
    let params = json_params("{}");
    let extra_headers = headers(&[("X-Trace", "trace-a")]);
    let mut setup = Setup::new("openai", &params);
    setup.after = Some("video_a");
    setup.limit = Some(2);
    setup.order = Some("desc");
    setup.extra_query = Some(&extra_query);
    setup.extra_headers = Some(&extra_headers);
    setup.timeout = Some(Duration::from_secs(5));
    let answer = run(&mut bench, setup, CallKind::List, CallMode::Asynchronous)
        .await
        .expect("the list handler answers");
    assert_eq!(answer.page(), bench.config.list_page().clone());
    assert_list_get(&bench, &wire, &extra_query);
    assert_list_pre_call(&bench);
}

fn assert_list_get(bench: &Bench<Source>, wire: &Wire, extra_query: &Params) {
    let sent = wire.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, Method::GET);
    assert_eq!(sent[0].url.as_str(), bench.config.list_url());
    assert_eq!(
        sent[0].query,
        vec![
            (String::from("after"), String::from("video_a")),
            (String::from("limit"), String::from("2")),
            (String::from("order"), String::from("desc")),
            (String::from("tag"), String::from("a")),
            (String::from("tag"), String::from("b")),
        ]
    );
    assert_eq!(sent[0].timeout, None);
    assert!(matches!(sent[0].body, RequestBody::Empty));
    assert_eq!(
        sent[0]
            .headers
            .get("x-trace")
            .and_then(|value| value.to_str().ok()),
        Some("trace-a")
    );
    assert_list_inputs(bench, extra_query);
}

fn assert_list_inputs(bench: &Bench<Source>, extra_query: &Params) {
    let inputs = bench.config.list_inputs();
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].after.as_deref(), Some("video_a"));
    assert_eq!(inputs[0].limit, Some(2));
    assert_eq!(inputs[0].order.as_deref(), Some("desc"));
    assert_eq!(inputs[0].extra_query.as_ref(), Some(extra_query));
    assert_eq!(inputs[0].api_base, bench.config.base());
    assert_eq!(
        inputs[0].headers.get("X-Trace").map(String::as_str),
        Some("trace-a")
    );
}

fn assert_list_pre_call(bench: &Bench<Source>) {
    assert_eq!(bench.fixture.input(), Some(&Json::Text(String::new())));
    assert_eq!(
        bench.fixture.logged("params"),
        Some(&Json::Object(bench.config.list_params().clone()))
    );
    assert_eq!(
        bench.fixture.logged("api_base"),
        Some(&Json::Text(bench.config.list_url().to_owned()))
    );
    assert!(bench.fixture.logged("complete_input_dict").is_none());
    assert!(bench.fixture.logged("video_id").is_none());
    assert_eq!(bench.events.recorded(), EVENTS);
}

// The list async path takes a supplied async client as it is: the unknown provider name is never parsed and the pool is never asked
async fn async_supplied_client() {
    let (mut bench, pooled) = fake_bench(LIST, Reply::json(200, "{}"), RecordingConfig::new);
    let supplied = Wire::replying(Reply::json(200, "{}"), &bench.events);
    let params = json_params("{}");
    let mut setup = Setup::new("nope", &params);
    setup.client = Some(VideoClient::Asynchronous(&supplied));
    let answer = run(&mut bench, setup, CallKind::List, CallMode::Asynchronous)
        .await
        .expect("the list handler answers");
    assert_eq!(answer.page(), bench.config.list_page().clone());
    assert!(pooled.requests().is_empty());
    assert!(bench.transport.source.async_calls().is_empty());
    let sent = supplied.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, Method::GET);
}

// The real B173 transport sends the query it was given, and the caller's timeout is not part of the GET
async fn real_get() {
    let serving = serve(
        reply(
            "200 fixture",
            &[("content-type", "application/json")],
            LIST_PAGE,
        ),
        Duration::ZERO,
    )
    .expect("the fixture serves");
    let address = serving.address;
    let configure = move |events: &Events| {
        RecordingConfig::new(events)
            .with_base(&format!("http://{address}/v1"))
            .with_list_url(&format!("http://{address}/v1/videos"))
            .with_list_params(json_params(
                r#"{"after":"video_a","limit":2,"order":"desc","include":"x"}"#,
            ))
    };
    let mut bench = proxied_bench(LIST, configure, TransportLimits::DEFAULT);
    let params = json_params("{}");
    let extra_query = json_params(r#"{"include":"x"}"#);
    let mut setup = Setup::new("openai", &params);
    setup.after = Some("video_a");
    setup.limit = Some(2);
    setup.order = Some("desc");
    setup.extra_query = Some(&extra_query);
    setup.timeout = Some(Duration::from_secs(5));
    let answer = run(&mut bench, setup, CallKind::List, CallMode::Asynchronous)
        .await
        .expect("the list handler answers");
    assert_eq!(answer.page(), bench.config.list_page().clone());
    let sent = serving.finish().expect("the fixture answers");
    assert_eq!(
        sent.line,
        "GET /v1/videos?after=video_a&limit=2&order=desc&include=x HTTP/1.1"
    );
}

// The sync handler runs the async one to completion: no api_key, a pooled async client, and a supplied sync client ignored
fn sync_runs_to_completion() {
    let (mut bench, pooled) = fake_bench(LIST, Reply::json(200, "{}"), RecordingConfig::new);
    let supplied = Wire::replying(Reply::json(200, "{}"), &bench.events);
    let params = json_params(r#"{"api_key":"sk-params"}"#);
    let argument = secret("sk-argument");
    let mut setup = Setup::new("openai", &params);
    setup.client = Some(VideoClient::Sync(&supplied));
    setup.api_key = Some(&argument);
    let answer = run_sync_list(&mut bench, setup).expect("the sync list handler answers");
    assert_eq!(answer, bench.config.list_page().clone());
    assert_eq!(bench.config.api_keys(), vec![None]);
    assert_eq!(
        bench.transport.source.async_calls(),
        vec![(ProviderId::Openai, None)]
    );
    assert!(bench.transport.source.sync_calls().is_empty());
    assert!(supplied.requests().is_empty());
    let sent = pooled.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, Method::GET);
}

// The _is_async branch is a plain hand-off, and it keeps the api_key the sync branch drops
async fn hand_off_forwards_api_key() {
    let (mut bench, _) = fake_bench(LIST, Reply::json(200, "{}"), RecordingConfig::new);
    let params = json_params("{}");
    let argument = secret("sk-argument");
    let mut setup = Setup::new("openai", &params);
    setup.api_key = Some(&argument);
    let answer = run(&mut bench, setup, CallKind::List, CallMode::HandOff)
        .await
        .expect("the hand-off runs the async handler");
    assert_eq!(answer.page(), bench.config.list_page().clone());
    assert_eq!(
        bench.config.api_keys(),
        vec![Some(String::from("sk-argument"))]
    );
    assert!(bench.transport.source.sync_calls().is_empty());
    assert_eq!(
        bench.transport.source.async_calls(),
        vec![(ProviderId::Openai, None)]
    );
}

// Inside a running runtime the sync handler returns a typed error before any client is picked
fn running_runtime() {
    let (mut bench, wire) = fake_bench(LIST, Reply::json(200, "{}"), RecordingConfig::new);
    let params = json_params("{}");
    let argument = secret("sk-argument");
    let mut setup = Setup::new("openai", &params);
    setup.api_key = Some(&argument);
    let entered = test_runtime();
    let error = entered.block_on(async {
        match run(&mut bench, setup, CallKind::List, CallMode::Sync).await {
            Ok(_) => panic!("the sync list handler refuses to run inside a runtime"),
            Err(error) => error,
        }
    });
    assert!(matches!(
        error,
        VideoRemixListTransportError::RunningRuntime { status_code: 500 }
    ));
    assert_eq!(
        error.to_string(),
        "video list sync handler cannot run inside a running runtime"
    );
    assert!(bench.config.events().is_empty());
    assert!(wire.requests().is_empty());
    assert!(bench.transport.source.async_calls().is_empty());
    assert!(bench.transport.source.sync_calls().is_empty());
}
