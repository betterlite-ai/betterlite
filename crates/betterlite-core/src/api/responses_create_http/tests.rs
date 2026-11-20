#![cfg(test)]

mod support;

use std::sync::atomic::Ordering;
use std::time::Duration;

use betterlite_models::decisions::Json as ModelJson;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use http::Method;
use indexmap::IndexMap;
use rstest::rstest;

use self::support::{
    Answer, Config, Dispatcher, NO_CALLBACKS, NO_RESOLVER, RESP_A, Reply, Sent, Source, Wire,
    agentic_callback, body_map, config_with, create_deps, execute, headers, hooks, json, models,
    object, params, pre_hook_callback, text_input,
};
use crate::api::responses_compact_http::error::Stage as FailStage;
use crate::api::responses_compact_http::service::{ClientKind, SharedSession};
use crate::api::responses_create_http::config::CreateParams;
use crate::api::responses_create_http::error::{CreateError, Stage as ErrorStage};
use crate::api::responses_create_http::service::{CreateCall, CreateLive, CreateOutcome};
use crate::api::responses_hook_policy::callbacks::{CallbackLists, CallbackValue};
use crate::api::responses_stream_driver::source::PayloadSource;

const HOOK_RESPONSE: &str = r#"{"id":"resp_hook","created_at":2,"object":"response","status":"completed","model":"gpt-4o","output":[]}"#;

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
    ]));
    let config = Config::default();
    let wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let source = Source::with(wire.clone());
    let mut call = create_call(&input, &optional, &call_params);
    call.extra_headers = Some(&extra_headers);
    call.extra_body = Some(&extra_body);
    let create_hooks = hooks(callbacks, &dispatcher, &NO_RESOLVER);
    let deps = create_deps(&config, &source);
    let outcome = execute(&deps, ClientKind::Sync, &call, &create_hooks)
        .await
        .expect("the sync call");

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
    assert_eq!(sent[0].method, Method::POST);
    assert_eq!(sent[0].url.as_str(), "http://mock/v1/responses");
    assert_eq!(sent[0].timeout, Duration::from_secs(7));
    assert_eq!(sent[0].header("authorization"), Some("Bearer explicit"));
    assert_eq!(sent[0].header("x-option"), Some("a"));
    assert_eq!(sent[0].header("x-trace"), Some("b"));
    assert_eq!(sent[0].header("x-signed"), Some("yes"));
    let fields = body_map(&sent[0].body_json()).clone();
    assert_eq!(fields.get("model"), Some(&json("\"gpt-4o-mini\"")));
    assert_eq!(fields.get("temperature"), Some(&json("0.8")));
    assert_eq!(fields.get("input"), Some(&json("\"hello\"")));
    assert!(!fields.contains_key("extra_headers"));
    assert!(!fields.contains_key("timeout"));
    assert!(!fields.contains_key("litellm_params"));
    assert!(!fields.contains_key("request_context"));
    assert_eq!(dispatcher.calls(), 1);
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
}

// One create call: its stream inputs, its callbacks and the transport's scripted answers
struct CallSetup<'a> {
    optional: &'a Object,
    body: Option<&'a Object>,
    fake: Option<&'a ModelJson>,
    metadata: Option<&'a IndexMap<String, ModelJson>>,
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
        CallbackPlan::Agentic(result) => vec![CallbackValue::Object(agentic_callback(result))],
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
        StreamExpect::Live => {
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
        StreamExpect::LiveGuard => {
            assert!(matches!(outcome, CreateOutcome::Live(_)));
            assert!(sent[0].stream);
        }
        StreamExpect::Synthetic => check_synthetic(outcome, &sent),
        StreamExpect::Plain => {
            assert_eq!(
                body_map(&read_value(&outcome)).get("id"),
                Some(&json("\"resp_a\""))
            );
            assert!(!sent[0].stream);
            assert!(body_map(&sent[0].body_json()).get("stream").is_none());
        }
    }
}

// A prepared fake stream drops the body's stream key and returns synthetic events with the request
async fn check_synthetic(outcome: CreateOutcome, sent: &[Sent]) {
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
#[case::neither_defaults_to_zero(SignCase::Timeout(None, r#"{}"#, Duration::ZERO))]
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
    let error = match execute(&deps, ClientKind::Sync, &call, &create_hooks).await {
        Ok(_) => panic!("the bad timeout fails"),
        Err(error) => error,
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
    execute(&deps, ClientKind::Sync, &call, &create_hooks)
        .await
        .expect("the signed call");
    let recorded = config.recorded_sign();
    assert!(!recorded.stream);
    assert!(matches!(recorded.fake_stream, Some(ModelJson::Bool(true))));
    assert_eq!(recorded.data.get("temperature"), Some(&json("0.8")));
    assert!(recorded.data.get("stream").is_none());
    assert_eq!(recorded.api_base, "http://mock/v1/responses");
    assert!(wire.requests()[0].body.is_empty());
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
    let (async_source, async_config) =
        shared_session_run(call_params, dispatcher, ClientKind::Asynchronous).await;
    assert_eq!(
        async_source.creations(),
        vec![(Some(false), Some(SharedSession(9)))]
    );
    assert!(async_config.off_loop_sign.load(Ordering::SeqCst));
    assert!(async_config.off_loop_transform.load(Ordering::SeqCst));

    let (sync_source, sync_config) =
        shared_session_run(call_params, dispatcher, ClientKind::Sync).await;
    assert_eq!(
        sync_source.creations(),
        vec![(Some(false), None::<SharedSession>)]
    );
    assert!(!sync_config.off_loop_sign.load(Ordering::SeqCst));
    assert!(!sync_config.off_loop_transform.load(Ordering::SeqCst));
}

async fn shared_session_run(
    call_params: &CreateParams,
    dispatcher: &Dispatcher,
    mode: ClientKind,
) -> (Source, Config) {
    let input = text_input("hello");
    let empty = Object::new();
    let create_hooks = hooks(NO_CALLBACKS, dispatcher, &NO_RESOLVER);
    let source = Source::with(Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]));
    let config = Config::default();
    let mut call = create_call(&input, &empty, call_params);
    call.timeout = Some(1.0_f64);
    call.shared_session = Some(SharedSession(9));
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

#[derive(Clone, Copy)]
enum FailureCase {
    Status,
    Step(FailStage),
    ContextUpdate,
}

#[rstest]
#[case::status(FailureCase::Status)]
#[case::transform(FailureCase::Step(FailStage::Transform))]
#[case::normalize(FailureCase::Step(FailStage::Normalize))]
#[case::sign(FailureCase::Step(FailStage::Sign))]
#[case::response_transform(FailureCase::Step(FailStage::Response))]
#[case::context_update(FailureCase::ContextUpdate)]
#[tokio::test]
async fn create_transport_failure(#[case] scenario: FailureCase) {
    let input = text_input("hello");
    let empty = Object::new();
    let call_params = params(r#"{"api_base":"http://mock/v1"}"#);
    let dispatcher = Dispatcher::quiet();
    match scenario {
        FailureCase::Status => {
            status_failure(&input, &empty, &call_params, &dispatcher).await;
        }
        FailureCase::Step(stage) => {
            step_failure(stage, &input, &empty, &call_params, &dispatcher).await;
        }
        FailureCase::ContextUpdate => context_update(&input, &call_params, &dispatcher).await,
    }
}

// HTTP 429 text=slow down with Retry-After=3 becomes B2402's 429 with the header preserved
async fn status_failure(
    input: &ResponsesInput,
    empty: &Object,
    call_params: &CreateParams,
    dispatcher: &Dispatcher,
) {
    let config = Config::default();
    let wire = Wire::new(vec![Answer::Json(
        Reply::json(429, "slow down").with_header("retry-after", "3"),
    )]);
    let source = Source::with(wire.clone());
    let call = create_call(input, empty, call_params);
    let create_hooks = hooks(NO_CALLBACKS, dispatcher, &NO_RESOLVER);
    let deps = create_deps(&config, &source);
    let error = match execute(&deps, ClientKind::Sync, &call, &create_hooks).await {
        Ok(_) => panic!("the 429 fails"),
        Err(error) => error,
    };
    let CreateError::Provider(provider) = error else {
        panic!("a 429 is a provider error")
    };
    assert_eq!(provider.status_code, 429);
    assert!(!provider.status_code_is_synthesized);
    assert_eq!(provider.message, "slow down");
    assert_eq!(provider.headers.get("retry-after"), Some(&models("\"3\"")));
    assert_eq!(wire.requests().len(), 1);
}

// transform, normalize and sign fail before the send catch and send nothing; the response transform runs after it
async fn step_failure(
    stage: FailStage,
    input: &ResponsesInput,
    empty: &Object,
    call_params: &CreateParams,
    dispatcher: &Dispatcher,
) {
    let config = config_with(Some(stage), None);
    let wire = Wire::new(if stage == FailStage::Response {
        vec![Answer::Json(Reply::json(200, RESP_A))]
    } else {
        Vec::new()
    });
    let source = Source::with(wire.clone());
    let call = create_call(input, empty, call_params);
    let create_hooks = hooks(NO_CALLBACKS, dispatcher, &NO_RESOLVER);
    let deps = create_deps(&config, &source);
    let error = match execute(&deps, ClientKind::Sync, &call, &create_hooks).await {
        Ok(_) => panic!("the step fails"),
        Err(error) => error,
    };
    match error {
        CreateError::Step { stage: found, .. } => assert_eq!(found, expected_stage(stage)),
        other => panic!("the failure is a step error: {other:?}"),
    }
    let sent = wire.requests();
    if stage == FailStage::Response {
        assert_eq!(sent.len(), 1);
    } else {
        assert!(sent.is_empty());
    }
}

fn expected_stage(stage: FailStage) -> ErrorStage {
    match stage {
        FailStage::Transform => ErrorStage::Transform,
        FailStage::Normalize => ErrorStage::Normalize,
        FailStage::Sign => ErrorStage::Sign,
        FailStage::Response => ErrorStage::Response,
        FailStage::Url | FailStage::Validate => panic!("only the transfer steps fail here"),
    }
}

// the optional map updates the request context first and generic params still land last
async fn context_update(
    input: &ResponsesInput,
    call_params: &CreateParams,
    dispatcher: &Dispatcher,
) {
    let optional = object(r#"{"stream":true,"input":"override","litellm_params":"ignored"}"#);
    let fake = models("true");
    let wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let source = Source::with(wire);
    let mut call = create_call(input, &optional, call_params);
    call.fake_stream = Some(&fake);
    let config = Config::default();
    let create_hooks = hooks(NO_CALLBACKS, dispatcher, &NO_RESOLVER);
    let deps = create_deps(&config, &source);
    let outcome = execute(&deps, ClientKind::Sync, &call, &create_hooks)
        .await
        .expect("the context call");
    let CreateOutcome::Synthetic(synthetic) = outcome else {
        panic!("the fake call returns synthetic events")
    };
    let context = synthetic.setup.request_data.expect("the request context");
    assert_eq!(context.get("input"), Some(&models("\"override\"")));
    assert_eq!(context.get("stream"), Some(&models("true")));
    assert!(matches!(
        context.get("litellm_params"),
        Some(ModelJson::Object(_))
    ));
}
