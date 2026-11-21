#![cfg(test)]

mod failures;
mod proxied;
mod support;

use std::fs;
use std::path::PathBuf;
use std::str::from_utf8;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use betterlite_models::decisions::Json as ModelJson;
use betterlite_openai::response_input::{ResponsesInput, ResponsesItem};
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use http::Method;
use indexmap::IndexMap;
use rstest::rstest;

use self::support::{
    AgenticRecorder, Answer, Config, Dispatcher, NO_CALLBACKS, NO_RESOLVER, RESP_A, Reply,
    ScriptedCall, Sent, Source, Wire, agentic_callback, body_map, capture_debug, config_with,
    create_deps, execute, execute_logged, headers, hooks, json, models, object, params,
    pre_hook_callback, text_input,
};
use super::convert::float_value;
use crate::api::responses_compact_http::service::{ClientKind, SharedSession};
use crate::api::responses_create_http::config::CreateParams;
use crate::api::responses_create_http::error::CreateError;
use crate::api::responses_create_http::service::{CreateCall, CreateLive, CreateOutcome};
use crate::api::responses_hook_policy::callbacks::{CallbackLists, CallbackValue};
use crate::api::responses_stream_driver::source::PayloadSource;

const HOOK_RESPONSE: &str = r#"{"id":"resp_hook","created_at":2,"object":"response","status":"completed","model":"gpt-4o","output":[]}"#;

// A source that never ends fails the read instead of growing the payload list without bound
const PAYLOAD_LIMIT: usize = 1000;

fn create_call<'call>(
    input: &'call ResponsesInput,
    optional_params: &'call Object,
    call_params: &'call CreateParams,
) -> CreateCall<'call> {
    CreateCall {
        model: "gpt-4o",
        input,
        optional_params,
        custom_llm_provider: "openai",
        params: call_params,
        extra_headers: None,
        extra_body: None,
        timeout: Some(7.0_f64),
        fake_stream: None,
        litellm_metadata: None,
        shared_session: None,
    }
}

fn read_value(outcome: &CreateOutcome) -> PromptJson {
    match outcome {
        CreateOutcome::Value(value) => json(value.get()),
        CreateOutcome::Live(_) => panic!("the call returns a stream handoff"),
        CreateOutcome::Synthetic(_) => panic!("the call returns synthetic events"),
    }
}

async fn collect_payloads(source: &mut Box<dyn PayloadSource>) -> Vec<String> {
    let mut payloads = Vec::new();
    while let Some(item) = source.next_payload().await {
        assert!(
            payloads.len() < PAYLOAD_LIMIT,
            "the source returns a finite number of payloads"
        );
        match item {
            Ok(bytes) => payloads.push(String::from_utf8(bytes).expect("the payload is UTF-8")),
            Err(error) => panic!("the payload reads: {error:?}"),
        }
    }
    payloads
}

#[tokio::test]
async fn create_transform_overlay_order() {
    let input = text_input("hello");
    let optional = object(
        r#"{"temperature":0.2,"extra_headers":{"Authorization":"Bearer optional","X-Option":"a"}}"#,
    );
    let extra_headers = headers(&[("Authorization", "Bearer explicit"), ("X-Trace", "b")]);
    let extra_body = object(r#"{"temperature":0.8}"#);
    let call_params = params(r#"{"api_base":"http://mock/v1","api_key":"sk-base"}"#);
    let globals = [CallbackValue::Object(pre_hook_callback())];
    let callbacks = CallbackLists {
        global: &globals,
        dynamic: &[],
    };
    let dispatcher = Dispatcher::patching(IndexMap::from([
        (String::from("model"), models("\"gpt-4o-mini\"")),
        (String::from("api_key"), models("\"sk-hooked\"")),
        (String::from("input"), models("\"changed\"")),
    ]));
    let config = Config::default();
    let wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let source = Source::with(wire.clone());
    let mut call = create_call(&input, &optional, &call_params);
    call.extra_headers = Some(&extra_headers);
    call.extra_body = Some(&extra_body);
    let create_hooks = hooks(callbacks, &dispatcher, &NO_RESOLVER);
    let deps = create_deps(&config, &source);
    let logged = execute_logged(&deps, ClientKind::Sync, &call, &create_hooks).await;
    let outcome = logged.outcome.expect("the sync call");

    // the pre-hook replaced the input, and pre_call logs what it replaced it with
    assert_eq!(logged.details.get("input"), Some(&models("\"changed\"")));
    assert_overlay_request(&wire, &dispatcher);
    assert_overlay_records(&config);
    assert_eq!(
        body_map(&read_value(&outcome)).get("id"),
        Some(&json("\"resp_a\""))
    );
    assert_async_keeps_original(&config, &call, callbacks).await;
}

fn assert_overlay_request(wire: &Wire, dispatcher: &Dispatcher) {
    let sent = wire.requests();
    assert_eq!(sent.len(), 1);
    assert_overlay_transport(&sent[0]);
    assert_overlay_body(&sent[0]);
    assert_eq!(dispatcher.calls(), 1);
}

fn assert_overlay_transport(sent: &Sent) {
    assert_eq!(sent.method, Method::POST);
    assert_eq!(sent.url.as_str(), "http://mock/v1/responses");
    assert_eq!(sent.timeout, Duration::from_secs(7));
    assert_eq!(sent.header("authorization"), Some("Bearer explicit"));
    assert_eq!(sent.header("x-option"), Some("a"));
    assert_eq!(sent.header("x-trace"), Some("b"));
    assert_eq!(sent.header("x-signed"), Some("yes"));
}

fn assert_overlay_body(sent: &Sent) {
    let fields = body_map(&sent.body_json()).clone();
    assert_eq!(fields.get("model"), Some(&json("\"gpt-4o-mini\"")));
    assert_eq!(fields.get("temperature"), Some(&json("0.8")));
    assert_eq!(fields.get("input"), Some(&json("\"changed\"")));
    assert!(!fields.contains_key("extra_headers"));
    assert!(!fields.contains_key("timeout"));
    assert!(!fields.contains_key("litellm_params"));
    assert!(!fields.contains_key("request_context"));
}

fn assert_overlay_records(config: &Config) {
    let validated = config.validated_headers();
    assert_eq!(validated.len(), 1);
    assert_eq!(
        validated[0].get("Authorization").map(String::as_str),
        Some("Bearer optional")
    );
    assert_eq!(validated[0].get("X-Option").map(String::as_str), Some("a"));
    let signed = config.recorded_sign();
    assert_eq!(signed.model, "gpt-4o-mini");
    assert_eq!(signed.api_key.as_deref(), Some("sk-hooked"));
    assert!(!signed.stream);
    assert_eq!(
        signed.headers.get("Authorization").map(String::as_str),
        Some("Bearer explicit")
    );
    assert_eq!(signed.data.get("temperature"), Some(&json("0.8")));
}

// async never reruns the sync deployment hook, so the original model and key stay
async fn assert_async_keeps_original(
    config: &Config,
    call: &CreateCall<'_>,
    callbacks: CallbackLists<'_>,
) {
    let dispatcher = Dispatcher::patching(IndexMap::from([(
        String::from("model"),
        models("\"gpt-4o-mini\""),
    )]));
    let wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let source = Source::with(wire.clone());
    let create_hooks = hooks(callbacks, &dispatcher, &NO_RESOLVER);
    let deps = create_deps(config, &source);
    execute(&deps, ClientKind::Asynchronous, call, &create_hooks)
        .await
        .expect("the async call");
    assert_eq!(dispatcher.calls(), 0);
    let sent = wire.requests();
    let body = body_map(&sent[0].body_json()).clone();
    assert_eq!(body.get("model"), Some(&json("\"gpt-4o\"")));
    assert_eq!(body.get("input"), Some(&json("\"hello\"")));
    assert_eq!(sent[0].header("authorization"), Some("Bearer explicit"));
    let signed = config.recorded_sign();
    assert_eq!(signed.model, "gpt-4o");
    assert_eq!(signed.api_key.as_deref(), Some("sk-base"));
}

// The stream inputs and the expected handoff of one create call
#[derive(Clone, Copy)]
struct StreamScenario {
    optional: &'static str,
    body: Option<&'static str>,
    fake: Option<&'static str>,
    metadata: bool,
    mode: ClientKind,
    sse: bool,
    expect: StreamExpect,
}

#[derive(Clone, Copy)]
enum StreamExpect {
    Live,
    LiveGuard,
    Synthetic,
    Plain,
    Unstreamed,
}

// One create call: its stream inputs, its callbacks and the transport's scripted answers
struct CallSetup<'call> {
    optional: &'call Object,
    body: Option<&'call Object>,
    fake: Option<&'call ModelJson>,
    metadata: Option<&'call IndexMap<String, ModelJson>>,
    callbacks: CallbackPlan,
    mode: ClientKind,
    answers: Vec<Answer>,
}

#[derive(Clone, Copy)]
enum CallbackPlan {
    None,
    Agentic(Option<&'static str>),
}

// Builds the scripted dependencies from the setup and runs the create call
async fn run_scenario(
    input: &ResponsesInput,
    call_params: &CreateParams,
    dispatcher: &Dispatcher,
    setup: CallSetup<'_>,
) -> (CreateOutcome, Wire) {
    let globals = match setup.callbacks {
        CallbackPlan::None => Vec::new(),
        CallbackPlan::Agentic(result) => {
            let recorder = Arc::new(AgenticRecorder::new());
            vec![CallbackValue::Object(agentic_callback(result, &recorder))]
        }
    };
    let callbacks = CallbackLists {
        global: &globals,
        dynamic: &[],
    };
    let wire = Wire::new(setup.answers);
    let source = Source::with(wire.clone());
    let mut call = create_call(input, setup.optional, call_params);
    call.extra_body = setup.body;
    call.fake_stream = setup.fake;
    call.litellm_metadata = setup.metadata;
    let create_hooks = hooks(callbacks, dispatcher, &NO_RESOLVER);
    let config = Config::default();
    let deps = create_deps(&config, &source);
    let outcome = execute(&deps, setup.mode, &call, &create_hooks)
        .await
        .expect("the scenario call");
    (outcome, wire)
}

#[rstest]
#[case::live_from_extra_body(StreamScenario {
    optional: r#"{"stream":false}"#,
    body: Some(r#"{"stream":true}"#),
    fake: None,
    metadata: true,
    mode: ClientKind::Asynchronous,
    sse: true,
    expect: StreamExpect::Live,
})]
#[case::fake_prepared(StreamScenario {
    optional: r#"{"stream":true}"#,
    body: Some(r#"{"stream":false}"#),
    fake: Some("true"),
    metadata: false,
    mode: ClientKind::Sync,
    sse: false,
    expect: StreamExpect::Synthetic,
})]
#[case::identity_guard(StreamScenario {
    optional: r#"{"stream":true}"#,
    body: None,
    fake: Some("1"),
    metadata: false,
    mode: ClientKind::Sync,
    sse: true,
    expect: StreamExpect::LiveGuard,
})]
#[case::plain_call(StreamScenario {
    optional: "{}",
    body: None,
    fake: Some("true"),
    metadata: false,
    mode: ClientKind::Sync,
    sse: false,
    expect: StreamExpect::Plain,
})]
#[case::falsy_stream_fake_keeps_body(StreamScenario {
    optional: r#"{"stream":false}"#,
    body: None,
    fake: Some("true"),
    metadata: false,
    mode: ClientKind::Sync,
    sse: false,
    expect: StreamExpect::Unstreamed,
})]
#[tokio::test]
async fn create_stream_and_fake_selection(#[case] scenario: StreamScenario) {
    let input = text_input("hello");
    let call_params = params(r#"{"api_base":"http://mock/v1"}"#);
    let dispatcher = Dispatcher::quiet();
    let optional = object(scenario.optional);
    let body = scenario.body.map(object);
    let fake = scenario.fake.map(models);
    let metadata = scenario
        .metadata
        .then(|| IndexMap::from([(String::from("trace"), models("\"t\""))]));
    let answers = if scenario.sse {
        vec![Answer::Sse(vec![
            String::from("hello"),
            String::from("[DONE]"),
        ])]
    } else {
        vec![Answer::Json(Reply::json(200, RESP_A))]
    };
    let setup = CallSetup {
        optional: &optional,
        body: body.as_ref(),
        fake: fake.as_ref(),
        metadata: metadata.as_ref(),
        callbacks: CallbackPlan::None,
        mode: scenario.mode,
        answers,
    };
    let (outcome, wire) = run_scenario(&input, &call_params, &dispatcher, setup).await;
    check_stream(scenario, outcome, &wire).await;
}

// The transport request and the handoff the scenario expects
async fn check_stream(scenario: StreamScenario, outcome: CreateOutcome, wire: &Wire) {
    let sent = wire.requests();
    match scenario.expect {
        StreamExpect::Live => check_live(outcome, &sent).await,
        StreamExpect::LiveGuard => check_live_guard(&outcome, &sent),
        StreamExpect::Synthetic => check_synthetic(outcome, &sent),
        StreamExpect::Plain => {
            assert_eq!(
                body_map(&read_value(&outcome)).get("id"),
                Some(&json("\"resp_a\""))
            );
            assert!(!sent[0].stream);
            assert!(body_map(&sent[0].body_json()).get("stream").is_none());
        }
        StreamExpect::Unstreamed => {
            assert_eq!(
                body_map(&read_value(&outcome)).get("id"),
                Some(&json("\"resp_a\""))
            );
            assert!(!sent[0].stream);
            assert_eq!(
                body_map(&sent[0].body_json()).get("stream"),
                Some(&json("false"))
            );
        }
    }
}

async fn check_live(outcome: CreateOutcome, sent: &[Sent]) {
    assert!(sent[0].stream);
    assert_eq!(
        body_map(&sent[0].body_json()).get("stream"),
        Some(&json("true"))
    );
    let CreateOutcome::Live(mut live) = outcome else {
        panic!("a streaming send returns the live iterator handoff")
    };
    assert_eq!(
        collect_payloads(&mut live.source).await,
        vec![String::from("hello"), String::from("[DONE]")]
    );
    assert_live_setup(&live);
}

fn check_live_guard(outcome: &CreateOutcome, sent: &[Sent]) {
    assert!(matches!(outcome, CreateOutcome::Live(_)));
    assert!(sent[0].stream);
}

// A prepared fake stream drops the body's stream key and returns synthetic events with the request
fn check_synthetic(outcome: CreateOutcome, sent: &[Sent]) {
    assert!(!sent[0].stream);
    assert!(body_map(&sent[0].body_json()).get("stream").is_none());
    let CreateOutcome::Synthetic(synthetic) = outcome else {
        panic!("a prepared fake stream returns the synthetic handoff")
    };
    assert_eq!(synthetic.response.id, "resp_a");
    assert_eq!(synthetic.setup.call_type.as_deref(), Some("responses"));
    assert_eq!(
        synthetic
            .setup
            .request_data
            .as_ref()
            .and_then(|context| context.get("input")),
        Some(&models("\"hello\""))
    );
}

// deviation: a 200 body without the typed fields fails the fake stream, where LiteLLM model_constructs it
#[tokio::test]
async fn create_lax_synthetic_body_is_not_typed() {
    let input = text_input("hello");
    let optional = object(r#"{"stream":true}"#);
    let call_params = params(r#"{"api_base":"http://mock/v1"}"#);
    let fake = models("true");
    let lax = r#"{"id":"resp_lax","created_at":1,"object":"response","status":500,"output":[]}"#;
    let wire = Wire::new(vec![Answer::Json(Reply::json(200, lax))]);
    let source = Source::with(wire.clone());
    let mut call = create_call(&input, &optional, &call_params);
    call.fake_stream = Some(&fake);
    let dispatcher = Dispatcher::quiet();
    let create_hooks = hooks(NO_CALLBACKS, &dispatcher, &NO_RESOLVER);
    let config = Config::default();
    let deps = create_deps(&config, &source);
    let Err(error) = execute(&deps, ClientKind::Sync, &call, &create_hooks).await else {
        panic!("the fake stream needs the typed response");
    };
    assert!(matches!(error, CreateError::NotTyped));
    assert_eq!(wire.requests().len(), 1);
}

// value_kind names Python's types, and the litellm_params type errors print them
#[test]
fn create_param_type_errors_name_the_python_type() {
    let ModelJson::Object(fields) = models(r#"{"api_base":7}"#) else {
        panic!("the params fixture is an object")
    };
    assert_eq!(
        CreateParams::from_map(fields)
            .expect_err("a number api_base")
            .to_string(),
        "litellm_params.api_base must be a string, got int"
    );
    let ModelJson::Object(verify_fields) = models(r#"{"ssl_verify":1.5}"#) else {
        panic!("the params fixture is an object")
    };
    assert_eq!(
        CreateParams::from_map(verify_fields)
            .expect_err("a float ssl_verify")
            .to_string(),
        "litellm_params.ssl_verify must be a boolean, got float"
    );
}

// The reply's HTTP headers reach transform_response_api_response, as LiteLLM hands them over
#[tokio::test]
async fn create_response_headers_reach_the_transform() {
    let input = text_input("hello");
    let optional = Object::new();
    let call_params = params(r#"{"api_base":"http://mock/v1"}"#);
    let dispatcher = Dispatcher::quiet();
    let config = Config::default();
    let answered = Reply::json(200, RESP_A).with_header("x-request-id", "r-1");
    let wire = Wire::new(vec![Answer::Json(answered)]);
    let source = Source::with(wire);
    let call = create_call(&input, &optional, &call_params);
    let create_hooks = hooks(NO_CALLBACKS, &dispatcher, &NO_RESOLVER);
    let deps = create_deps(&config, &source);
    execute(&deps, ClientKind::Sync, &call, &create_hooks)
        .await
        .expect("the reply transforms");
    assert_eq!(
        config.transformed_headers(),
        vec![IndexMap::from([(
            String::from("x-request-id"),
            String::from("r-1")
        )])]
    );
}

fn assert_live_setup(live: &CreateLive) {
    assert_eq!(live.setup.model, "gpt-4o");
    assert_eq!(live.setup.custom_llm_provider, "openai");
    assert_eq!(live.setup.call_type.as_deref(), Some("responses"));
    assert_eq!(
        live.setup
            .litellm_metadata
            .as_ref()
            .and_then(|known| known.get("trace")),
        Some(&models("\"t\""))
    );
    let context = live
        .setup
        .request_data
        .as_ref()
        .expect("the request context");
    assert_eq!(context.get("input"), Some(&models("\"hello\"")));
    assert_eq!(context.get("stream"), Some(&models("false")));
    assert!(matches!(
        context.get("litellm_params"),
        Some(ModelJson::Object(_))
    ));
}

#[derive(Clone, Copy)]
enum SignCase {
    Timeout(Option<f64>, &'static str, Duration),
    BadTimeout,
    SignedBody,
    ClientSelection,
    SharedSession,
}

#[rstest]
#[case::explicit_wins(SignCase::Timeout(
    Some(7.0_f64),
    r#"{"timeout":"9"}"#,
    Duration::from_secs(7)
))]
#[case::explicit_zero_falls_through(SignCase::Timeout(
    Some(0.0_f64),
    r#"{"timeout":"9"}"#,
    Duration::from_secs(9)
))]
#[case::neither_defaults_to_zero(SignCase::Timeout(None, "{}", Duration::ZERO))]
#[case::optional_number(SignCase::Timeout(
    None,
    r#"{"timeout":3.5}"#,
    Duration::from_millis(3500)
))]
#[case::optional_infinity(SignCase::Timeout(None, r#"{"timeout":"inf"}"#, Duration::MAX))]
#[case::bad_timeout(SignCase::BadTimeout)]
#[case::signed_body(SignCase::SignedBody)]
#[case::client_selection(SignCase::ClientSelection)]
#[case::shared_session(SignCase::SharedSession)]
#[tokio::test]
async fn create_sign_timeout_and_client(#[case] scenario: SignCase) {
    let call_params = params(r#"{"api_base":"http://mock/v1","ssl_verify":false}"#);
    let dispatcher = Dispatcher::quiet();
    match scenario {
        SignCase::Timeout(explicit, optional_text, expected) => {
            timeout_request(explicit, optional_text, expected, &call_params, &dispatcher).await;
        }
        SignCase::BadTimeout => bad_timeout(&call_params, &dispatcher).await,
        SignCase::SignedBody => signed_body(&call_params, &dispatcher).await,
        SignCase::ClientSelection => client_selection(&call_params, &dispatcher).await,
        SignCase::SharedSession => shared_session(&call_params, &dispatcher).await,
    }
}

async fn timeout_request(
    explicit: Option<f64>,
    optional_text: &str,
    expected: Duration,
    call_params: &CreateParams,
    dispatcher: &Dispatcher,
) {
    let input = text_input("hello");
    let optional = object(optional_text);
    let wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let source = Source::with(wire.clone());
    let mut call = create_call(&input, &optional, call_params);
    call.timeout = explicit;
    let create_hooks = hooks(NO_CALLBACKS, dispatcher, &NO_RESOLVER);
    let config = Config::default();
    let deps = create_deps(&config, &source);
    execute(&deps, ClientKind::Sync, &call, &create_hooks)
        .await
        .expect("the timed call");
    assert_eq!(wire.requests()[0].timeout, expected, "{optional_text}");
}

// a timeout that float() rejects becomes B2402's synthesized 500 before any HTTP request
async fn bad_timeout(call_params: &CreateParams, dispatcher: &Dispatcher) {
    let input = text_input("hello");
    let optional = object(r#"{"timeout":"abc"}"#);
    let wire = Wire::new(Vec::new());
    let source = Source::with(wire.clone());
    let config = Config::default();
    let mut call = create_call(&input, &optional, call_params);
    call.timeout = None;
    let create_hooks = hooks(NO_CALLBACKS, dispatcher, &NO_RESOLVER);
    let deps = create_deps(&config, &source);
    let Err(error) = execute(&deps, ClientKind::Sync, &call, &create_hooks).await else {
        panic!("the bad timeout fails");
    };
    let CreateError::Provider(provider) = error else {
        panic!("the timeout conversion failure is a provider error")
    };
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert_eq!(provider.message, "could not convert string to float: 'abc'");
    assert!(wire.requests().is_empty());
}

// signing sees the post-overlay, post-fake-prep body; empty signed bytes stay the whole body
async fn signed_body(call_params: &CreateParams, dispatcher: &Dispatcher) {
    let input = text_input("hello");
    let config = config_with(None, Some(Vec::new()));
    let optional = object(r#"{"stream":true}"#);
    let body = object(r#"{"temperature":0.8}"#);
    let fake = models("true");
    let wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let source = Source::with(wire.clone());
    let mut call = create_call(&input, &optional, call_params);
    call.extra_body = Some(&body);
    call.fake_stream = Some(&fake);
    call.timeout = Some(1.0_f64);
    let create_hooks = hooks(NO_CALLBACKS, dispatcher, &NO_RESOLVER);
    let deps = create_deps(&config, &source);
    let logged = execute_logged(&deps, ClientKind::Sync, &call, &create_hooks).await;
    logged.outcome.expect("the signed call");
    let recorded = config.recorded_sign();
    assert!(!recorded.stream);
    assert!(matches!(recorded.fake_stream, Some(ModelJson::Bool(true))));
    assert_eq!(recorded.data.get("temperature"), Some(&json("0.8")));
    assert!(recorded.data.get("stream").is_none());
    assert_eq!(recorded.api_base, "http://mock/v1/responses");
    assert!(wire.requests()[0].body.is_empty());
    assert_pre_call_record(&logged.details);
}

// _pre_call logs the original input, api_key="", the final body, the base URL and the signed headers
fn assert_pre_call_record(details: &IndexMap<String, ModelJson>) {
    assert_eq!(details.get("input"), Some(&models("\"hello\"")));
    assert_eq!(details.get("api_key"), Some(&models("\"\"")));
    let Some(ModelJson::Object(args)) = details.get("additional_args") else {
        panic!("pre_call logs its additional arguments");
    };
    assert_eq!(
        args.get("api_base"),
        Some(&models("\"http://mock/v1/responses\""))
    );
    assert_eq!(
        args.get("complete_input_dict"),
        Some(&models(
            r#"{"model":"gpt-4o","input":"hello","temperature":0.8}"#
        ))
    );
    let Some(ModelJson::Object(logged_headers)) = args.get("headers") else {
        panic!("pre_call logs the signed headers");
    };
    assert_eq!(logged_headers.get("X-Signed"), Some(&models("\"yes\"")));
    assert_eq!(
        logged_headers.get("Content-Type"),
        Some(&models("\"application/json\""))
    );
}

// a supplied client of the matching kind is reused; the wrong kind selects the pool
async fn client_selection(call_params: &CreateParams, dispatcher: &Dispatcher) {
    let input = text_input("hello");
    let empty = Object::new();
    let config = Config::default();
    let reuse_wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let reuse_pool = Wire::new(Vec::new());
    let reuse_source = Source::with_supplied(reuse_pool, ClientKind::Sync, reuse_wire.clone());
    let mut call = create_call(&input, &empty, call_params);
    call.timeout = Some(1.0_f64);
    let create_hooks = hooks(NO_CALLBACKS, dispatcher, &NO_RESOLVER);
    let reuse_deps = create_deps(&config, &reuse_source);
    execute(&reuse_deps, ClientKind::Sync, &call, &create_hooks)
        .await
        .expect("the supplied client call");
    assert_eq!(reuse_wire.requests().len(), 1);
    assert!(reuse_source.creations().is_empty());

    let wrong_wire = Wire::new(Vec::new());
    let wrong_pool = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let wrong_source =
        Source::with_supplied(wrong_pool.clone(), ClientKind::Sync, wrong_wire.clone());
    let wrong_deps = create_deps(&config, &wrong_source);
    execute(&wrong_deps, ClientKind::Asynchronous, &call, &create_hooks)
        .await
        .expect("the wrong-kind call");
    assert!(wrong_wire.requests().is_empty());
    assert_eq!(wrong_pool.requests().len(), 1);
}

// async forwards ssl_verify and shared_session to the pool and signs off the loop; sync forwards neither
async fn shared_session(call_params: &CreateParams, dispatcher: &Dispatcher) {
    assert_async_forwards_session(call_params, dispatcher, Some(SharedSession(9)), "9").await;
    assert_async_forwards_session(call_params, dispatcher, None, "None").await;
    assert_reused_client_skips_debug(call_params, dispatcher).await;
    assert_sync_forwards_nothing(call_params, dispatcher).await;
}

async fn assert_async_forwards_session(
    call_params: &CreateParams,
    dispatcher: &Dispatcher,
    session: Option<SharedSession>,
    logged: &str,
) {
    let capture = capture_debug();
    let (source, config) =
        shared_session_run(call_params, dispatcher, ClientKind::Asynchronous, session).await;
    assert_eq!(source.creations(), vec![(Some(false), session)]);
    assert!(config.off_loop_sign.load(Ordering::SeqCst));
    assert!(config.off_loop_transform.load(Ordering::SeqCst));
    let line = format!("Creating HTTP client for responses API with shared_session: {logged}");
    assert!(capture.logs.take().iter().any(|seen| *seen == line));
}

async fn assert_sync_forwards_nothing(call_params: &CreateParams, dispatcher: &Dispatcher) {
    let capture = capture_debug();
    let pooled = vec![(Some(false), None::<SharedSession>)];
    let (source, config) = shared_session_run(
        call_params,
        dispatcher,
        ClientKind::Sync,
        Some(SharedSession(9)),
    )
    .await;
    assert_eq!(source.creations(), pooled);
    assert!(!config.off_loop_sign.load(Ordering::SeqCst));
    assert!(!config.off_loop_transform.load(Ordering::SeqCst));
    assert!(
        !capture
            .logs
            .take()
            .iter()
            .any(|line| line.starts_with("Creating HTTP client"))
    );
}

// a supplied client of the matching async kind never builds one, so the debug line never runs
async fn assert_reused_client_skips_debug(call_params: &CreateParams, dispatcher: &Dispatcher) {
    let input = text_input("hello");
    let empty = Object::new();
    let supplied = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let pool = Wire::new(Vec::new());
    let source = Source::with_supplied(pool, ClientKind::Asynchronous, supplied);
    let config = Config::default();
    let mut call = create_call(&input, &empty, call_params);
    call.timeout = Some(1.0_f64);
    call.shared_session = Some(SharedSession(9));
    let create_hooks = hooks(NO_CALLBACKS, dispatcher, &NO_RESOLVER);
    let deps = create_deps(&config, &source);
    let capture = capture_debug();
    execute(&deps, ClientKind::Asynchronous, &call, &create_hooks)
        .await
        .expect("the supplied async client");
    assert!(
        capture
            .logs
            .take()
            .iter()
            .all(|line| line != "Creating HTTP client for responses API with shared_session: 9")
    );
}

async fn shared_session_run(
    call_params: &CreateParams,
    dispatcher: &Dispatcher,
    mode: ClientKind,
    session: Option<SharedSession>,
) -> (Source, Config) {
    let input = text_input("hello");
    let empty = Object::new();
    let create_hooks = hooks(NO_CALLBACKS, dispatcher, &NO_RESOLVER);
    let source = Source::with(Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]));
    let config = Config::default();
    let mut call = create_call(&input, &empty, call_params);
    call.timeout = Some(1.0_f64);
    call.shared_session = session;
    let deps = create_deps(&config, &source);
    execute(&deps, mode, &call, &create_hooks)
        .await
        .expect("the client call");
    (source, config)
}

#[derive(Clone, Copy)]
enum AgenticExpect {
    Wrapped(&'static str),
    Plain(&'static str),
    Value(&'static str),
}

// One agentic call: its generic params, the scripted hook and the result the wrap decision expects
#[derive(Clone, Copy)]
struct AgenticScenario {
    call_params: &'static str,
    hook: CallbackPlan,
    mode: ClientKind,
    expect: AgenticExpect,
}

#[rstest]
#[case::websearch(AgenticScenario {
    call_params: r#"{"api_base":"http://mock/v1","_websearch_interception_converted_stream":true}"#,
    hook: CallbackPlan::Agentic(Some(HOOK_RESPONSE)),
    mode: ClientKind::Asynchronous,
    expect: AgenticExpect::Wrapped("resp_hook"),
})]
#[case::compression(AgenticScenario {
    call_params: r#"{"api_base":"http://mock/v1","_compression_interception_converted_stream":true}"#,
    hook: CallbackPlan::Agentic(Some(HOOK_RESPONSE)),
    mode: ClientKind::Asynchronous,
    expect: AgenticExpect::Wrapped("resp_hook"),
})]
#[case::headroom(AgenticScenario {
    call_params: r#"{"api_base":"http://mock/v1","_headroom_interception_converted_stream":true}"#,
    hook: CallbackPlan::Agentic(Some(HOOK_RESPONSE)),
    mode: ClientKind::Asynchronous,
    expect: AgenticExpect::Wrapped("resp_hook"),
})]
#[case::code_interpreter(AgenticScenario {
    call_params: r#"{"api_base":"http://mock/v1","_code_interpreter_interception_converted_stream":true}"#,
    hook: CallbackPlan::Agentic(Some(HOOK_RESPONSE)),
    mode: ClientKind::Asynchronous,
    expect: AgenticExpect::Wrapped("resp_hook"),
})]
#[case::null_result(AgenticScenario {
    call_params: r#"{"api_base":"http://mock/v1","_websearch_interception_converted_stream":true}"#,
    hook: CallbackPlan::Agentic(None),
    mode: ClientKind::Asynchronous,
    expect: AgenticExpect::Wrapped("resp_a"),
})]
#[case::falsy_flag(AgenticScenario {
    call_params: r#"{"api_base":"http://mock/v1","_headroom_interception_converted_stream":""}"#,
    hook: CallbackPlan::Agentic(None),
    mode: ClientKind::Asynchronous,
    expect: AgenticExpect::Plain("\"resp_a\""),
})]
#[case::depth(AgenticScenario {
    call_params: r#"{"api_base":"http://mock/v1","_websearch_interception_converted_stream":true,"_agentic_loop_depth":1}"#,
    hook: CallbackPlan::Agentic(Some(HOOK_RESPONSE)),
    mode: ClientKind::Sync,
    expect: AgenticExpect::Plain("\"resp_hook\""),
})]
#[case::depth_zero(AgenticScenario {
    call_params: r#"{"api_base":"http://mock/v1","_websearch_interception_converted_stream":true,"_agentic_loop_depth":0}"#,
    hook: CallbackPlan::Agentic(Some(HOOK_RESPONSE)),
    mode: ClientKind::Sync,
    expect: AgenticExpect::Wrapped("resp_hook"),
})]
#[case::false_result(AgenticScenario {
    call_params: r#"{"api_base":"http://mock/v1"}"#,
    hook: CallbackPlan::Agentic(Some("false")),
    mode: ClientKind::Asynchronous,
    expect: AgenticExpect::Value("false"),
})]
#[case::no_hook_sync(AgenticScenario {
    call_params: r#"{"api_base":"http://mock/v1","_websearch_interception_converted_stream":true}"#,
    hook: CallbackPlan::None,
    mode: ClientKind::Sync,
    expect: AgenticExpect::Plain("\"resp_a\""),
})]
#[case::no_hook_async(AgenticScenario {
    call_params: r#"{"api_base":"http://mock/v1","_websearch_interception_converted_stream":true}"#,
    hook: CallbackPlan::None,
    mode: ClientKind::Asynchronous,
    expect: AgenticExpect::Wrapped("resp_a"),
})]
#[tokio::test]
async fn create_agentic_result_wrapping(#[case] scenario: AgenticScenario) {
    let input = text_input("hello");
    let dispatcher = Dispatcher::quiet();
    let empty = Object::new();
    let call_params = params(scenario.call_params);
    let setup = CallSetup {
        optional: &empty,
        body: None,
        fake: None,
        metadata: None,
        callbacks: scenario.hook,
        mode: scenario.mode,
        answers: vec![Answer::Json(Reply::json(200, RESP_A))],
    };
    let (outcome, _) = run_scenario(&input, &call_params, &dispatcher, setup).await;
    check_agentic(scenario.expect, outcome);
}

// A non-null result replaces the initial response; a null result keeps it and the flag decides the wrap
fn check_agentic(expect: AgenticExpect, outcome: CreateOutcome) {
    match expect {
        AgenticExpect::Wrapped(id) => {
            let CreateOutcome::Synthetic(synthetic) = outcome else {
                panic!("the converted flag wraps the result as synthetic events")
            };
            assert_eq!(synthetic.response.id, id);
            assert_eq!(synthetic.setup.model, "gpt-4o");
            assert!(synthetic.setup.request_data.is_none());
            assert!(synthetic.setup.litellm_metadata.is_none());
            assert!(synthetic.setup.call_type.is_none());
        }
        AgenticExpect::Plain(id) => {
            assert_eq!(body_map(&read_value(&outcome)).get("id"), Some(&json(id)));
        }
        AgenticExpect::Value(text) => assert_eq!(read_value(&outcome), json(text)),
    }
}

// The agentic hooks see the input list, or one user turn for a text input, as LiteLLM's messages argument
#[rstest]
#[case::text_followup(false)]
#[case::items_followup(true)]
#[tokio::test]
async fn create_agentic_followup_messages(#[case] items: bool) {
    let text = text_input("hello");
    let item = json(r#"{"type":"message","role":"user","content":"hi"}"#);
    let listed = ResponsesInput::Items(vec![ResponsesItem(item.clone())]);
    let input = if items { &listed } else { &text };
    let expected = if items {
        item
    } else {
        json(r#"{"role":"user","content":"hello"}"#)
    };
    let recorder = Arc::new(AgenticRecorder::new());
    let globals = [CallbackValue::Object(agentic_callback(
        Some(HOOK_RESPONSE),
        &recorder,
    ))];
    let callbacks = CallbackLists {
        global: &globals,
        dynamic: &[],
    };
    let optional = object(r#"{"temperature":0.2}"#);
    let call_params =
        params(r#"{"api_base":"http://mock/v1","_websearch_interception_converted_stream":true}"#);
    let dispatcher = Dispatcher::quiet();
    let config = Config::default();
    let source = Source::with(Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]));
    let call = create_call(input, &optional, &call_params);
    let create_hooks = hooks(callbacks, &dispatcher, &NO_RESOLVER);
    let deps = create_deps(&config, &source);
    execute(&deps, ClientKind::Asynchronous, &call, &create_hooks)
        .await
        .expect("the agentic call");
    assert_eq!(
        recorder.calls(),
        vec![
            ScriptedCall::Gate(vec![expected.clone()]),
            ScriptedCall::Loop(vec![expected]),
        ]
    );
    assert_eq!(recorder.optional(), vec![json(r#"{"temperature":0.2}"#)]);
}

// The timeout text parser's seeds, replayed through the same float() the handler calls
#[rstest]
fn replays_the_create_timeout_text_corpus(
    #[files("fuzz/corpus/create_timeout_text/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("the corpus file");
    let Ok(text) = from_utf8(&bytes) else {
        return;
    };
    drop(float_value(&PromptJson::Text(text.to_owned())));
}
