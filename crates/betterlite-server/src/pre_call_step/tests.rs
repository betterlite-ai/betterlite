#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::fallback_errors::FallbackErrors;
use betterlite_config::sections::pre_call::Settings as PreCallSection;
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::api::request_destination::model::ModelDefaults;
use betterlite_logging::callback_logger::resolver::LoggerResolver;
use betterlite_logging::datadog::apm::{NullTracer, Tracer};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object as JsonMap;
use http::Request;
use http::request::Parts;
use rstest::rstest;

use super::standalone::{AllowAllBudgets, NoRouterSettings};
use super::{PreCall, PreCallError, PreCallInput, PreCallOutput, PreCallSettings, PreCallStep};
use crate::pre_call::failure::{PreCallFailure, ProxyFailure};
use crate::pre_call::key::PreCallKey;
use crate::pre_call::ports::{
    CompressionArming, GuardrailMerge, PreCallHooks, PreCallPorts, RequestPreparer,
    RetrievalPipelines, RouterSettingsSource, RouterView, StreamOptionsCheck, TagBudgets,
    VectorStoreAccess,
};
use crate::pre_call::rewrites::{CliOverrides, RequestModel};
use crate::pre_call::route_type::RouteType;

mod support;

use support::{Fake, NoLoggers, TestLoggers, TestTracer, rate_limited};

#[derive(Default)]
struct TestKey {
    aliases: Option<JsonMap>,
    router_settings: Option<JsonValue>,
    team: Option<String>,
}

impl PreCallKey for TestKey {
    fn team_id(&self) -> Option<&str> {
        self.team.as_deref()
    }

    fn key_aliases(&self) -> Option<&JsonMap> {
        self.aliases.as_ref()
    }

    fn key_router_settings(&self) -> Option<&JsonValue> {
        self.router_settings.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Prepare,
    Access(String),
    Settings,
    Fallbacks,
    FallbackModels(String, Vec<String>),
    Merge(Option<String>),
    Compress,
    Hook(bool),
    Skip(String),
    Tags(Vec<String>),
    Attach,
    Prefetch(String),
    Authorize(String),
}

struct Behavior {
    prepare: Option<PreCallFailure>,
    settings: Option<JsonMap>,
    fallbacks: Option<Vec<JsonValue>>,
    fallback_models: Option<Vec<String>>,
    supports: bool,
    skip_budget: bool,
    hook_failure: Option<PreCallFailure>,
    hook_tags: Vec<String>,
    rate_limited_models: Vec<String>,
    authorize: Option<PreCallFailure>,
    access: Option<PreCallFailure>,
    tag_on_prepare: Option<String>,
}

fn behavior() -> Behavior {
    Behavior {
        prepare: None,
        settings: None,
        fallbacks: None,
        fallback_models: None,
        supports: true,
        skip_budget: true,
        hook_failure: None,
        hook_tags: Vec::new(),
        rate_limited_models: Vec::new(),
        authorize: None,
        access: None,
        tag_on_prepare: None,
    }
}

fn fallback_script(models: &[&str], limited: &[&str]) -> Behavior {
    Behavior {
        fallbacks: Some(
            models
                .iter()
                .map(|model| JsonValue::String((*model).to_owned()))
                .collect(),
        ),
        fallback_models: Some(models.iter().map(|model| (*model).to_owned()).collect()),
        rate_limited_models: limited.iter().map(|model| (*model).to_owned()).collect(),
        ..behavior()
    }
}

fn boom() -> PreCallFailure {
    PreCallFailure::Proxy(ProxyFailure {
        message: String::from("boom"),
        error_type: String::from("internal_server_error"),
        param: None,
        code: 500,
    })
}

fn data_from(source: &str) -> JsonMap {
    serde_json::from_str::<JsonMap>(source).expect("the fixture is valid JSON")
}

fn parts(path: &str, call_id: Option<&str>) -> Parts {
    let mut builder = Request::builder().uri(path);
    if let Some(call_id) = call_id {
        builder = builder.header("x-litellm-call-id", call_id);
    }
    let request = builder.body(()).unwrap();
    request.into_parts().0
}

fn text_at<'data>(data: &'data JsonMap, field: &str) -> &'data str {
    data.get(field)
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
}

fn call_input<'input>(
    data: JsonMap,
    request: &'input Parts,
    key: &'input TestKey,
    route_type: RouteType,
) -> PreCallInput<'input, TestKey> {
    PreCallInput {
        data,
        request,
        key,
        route_type,
        version: None,
        model: None,
        skip_guardrails: false,
    }
}

fn input_for<'input>(
    fixture: &str,
    request: &'input Parts,
    key: &'input TestKey,
    route_type: RouteType,
) -> PreCallInput<'input, TestKey> {
    let data = data_from(fixture);
    call_input(data, request, key, route_type)
}

fn ports(fake: &Arc<Fake>, tracer: Arc<dyn Tracer + Send + Sync>) -> PreCallPorts<TestKey> {
    let preparer: Arc<dyn RequestPreparer<TestKey>> = Arc::clone(fake);
    let vector_stores: Arc<dyn VectorStoreAccess<TestKey>> = Arc::clone(fake);
    let router_settings: Arc<dyn RouterSettingsSource<TestKey>> = Arc::clone(fake);
    let router: Arc<dyn RouterView<TestKey>> = Arc::clone(fake);
    let stream_options: Arc<dyn StreamOptionsCheck> = Arc::clone(fake);
    let guardrail_merge: Arc<dyn GuardrailMerge> = Arc::clone(fake);
    let compression: Arc<dyn CompressionArming> = Arc::clone(fake);
    let hooks: Arc<dyn PreCallHooks<TestKey>> = Arc::clone(fake);
    let tag_budgets: Arc<dyn TagBudgets<TestKey>> = Arc::clone(fake);
    let retrieval: Arc<dyn RetrievalPipelines<TestKey>> = Arc::clone(fake);
    PreCallPorts {
        preparer,
        vector_stores,
        router_settings,
        router: Some(router),
        stream_options,
        guardrail_merge,
        compression,
        hooks,
        tag_budgets,
        retrieval,
        tracer,
    }
}

fn ports_without_router(fake: &Arc<Fake>) -> PreCallPorts<TestKey> {
    let tracer: Arc<dyn Tracer + Send + Sync> = Arc::new(NullTracer);
    let mut built = ports(fake, tracer);
    built.router = None;
    built
}

fn step_full(
    fake: &Arc<Fake>,
    tracer: Arc<dyn Tracer + Send + Sync>,
    loggers: Arc<dyn LoggerResolver>,
    settings: PreCallSettings,
) -> PreCallStep<TestKey> {
    PreCallStep::new(ports(fake, tracer), settings, loggers)
}

fn step_for(fake: &Arc<Fake>) -> PreCallStep<TestKey> {
    let tracer: Arc<dyn Tracer + Send + Sync> = Arc::new(NullTracer);
    let loggers: Arc<dyn LoggerResolver> = Arc::new(NoLoggers);
    let settings = PreCallSettings::default();
    step_full(fake, tracer, loggers, settings)
}

fn step_without_router(fake: &Arc<Fake>) -> PreCallStep<TestKey> {
    let settings = PreCallSettings::default();
    let loggers: Arc<dyn LoggerResolver> = Arc::new(NoLoggers);
    PreCallStep::new(ports_without_router(fake), settings, loggers)
}

fn settings_with(overrides: CliOverrides, pre_call: PreCallSection) -> PreCallSettings {
    PreCallSettings {
        fallback_errors: FallbackErrors::default(),
        pre_call,
        overrides,
        user_model: None,
        model_defaults: ModelDefaults::default(),
    }
}

fn hook_count(calls: &[Call]) -> usize {
    calls
        .iter()
        .filter(|call| matches!(call, Call::Hook(_)))
        .count()
}

fn merge_calls(calls: &[Call]) -> Vec<Call> {
    calls
        .iter()
        .filter(|call| matches!(call, Call::Merge(_)))
        .cloned()
        .collect()
}

fn access_calls(calls: &[Call]) -> Vec<Call> {
    calls
        .iter()
        .filter(|call| matches!(call, Call::Access(_)))
        .cloned()
        .collect()
}

async fn stores_for(route_type: RouteType) -> Vec<Call> {
    let fake = Arc::new(Fake::new(behavior()));
    let step = step_for(&fake);
    let request = parts("/v1/responses", None);
    let key = TestKey::default();
    let fixture =
        r#"{"model": "gpt", "tools": [{"type": "file_search", "vector_store_ids": ["b", "a"]}]}"#;
    let input = input_for(fixture, &request, &key, route_type);
    step.run(input.data.clone(), &input, None).await.unwrap();
    access_calls(&fake.events())
}

async fn stream_data(supports: bool) -> JsonMap {
    let script = Behavior {
        supports,
        ..behavior()
    };
    let fake = Arc::new(Fake::new(script));
    let step = step_for(&fake);
    let request = parts("/chat/completions", None);
    let key = TestKey::default();
    let fixture = r#"{"model": "gpt", "stream": true}"#;
    let input = input_for(fixture, &request, &key, RouteType::Completion);
    let output = step.run(input.data.clone(), &input, None).await.unwrap();
    output.data
}

async fn merges_for(rate_limited_model: Option<&str>) -> Vec<Call> {
    let fake = Arc::new(Fake::new(behavior()));
    let step = step_for(&fake);
    let request = parts("/chat/completions", None);
    let key = TestKey::default();
    let fixture = r#"{"model": "gpt"}"#;
    let input = input_for(fixture, &request, &key, RouteType::Completion);
    step.run(input.data.clone(), &input, rate_limited_model)
        .await
        .unwrap();
    merge_calls(&fake.events())
}

async fn tag_events(skip_budget: bool) -> Vec<Call> {
    let script = Behavior {
        skip_budget,
        hook_tags: vec![String::from("added")],
        ..behavior()
    };
    let fake = Arc::new(Fake::new(script));
    let step = step_for(&fake);
    let request = parts("/chat/completions", None);
    let key = TestKey::default();
    let fixture = r#"{"model": "gpt", "metadata": {"tags": ["first"]}}"#;
    let input = input_for(fixture, &request, &key, RouteType::Completion);
    step.run(input.data.clone(), &input, None).await.unwrap();
    fake.events()
}

async fn attach_events(route_type: RouteType) -> Vec<Call> {
    let fake = Arc::new(Fake::new(behavior()));
    let step = step_for(&fake);
    let request = parts("/v1/responses/resp_1", None);
    let key = TestKey::default();
    let fixture = r#"{"model": "gpt"}"#;
    let input = input_for(fixture, &request, &key, route_type);
    step.run(input.data.clone(), &input, None).await.unwrap();
    fake.events()
}

async fn fallback_run(
    script: Behavior,
    fixture: &str,
    key: TestKey,
) -> (Vec<Call>, Result<PreCallOutput, PreCallError>) {
    let fake = Arc::new(Fake::new(script));
    let step = step_for(&fake);
    let request = parts("/chat/completions", None);
    let input = input_for(fixture, &request, &key, RouteType::Completion);
    let result = step.pre_call(input).await;
    let events = fake.events();
    (events, result)
}

#[tokio::test]
async fn step_runs_the_ports_in_order() {
    let fake = Arc::new(Fake::new(behavior()));
    let step = step_for(&fake);
    let request = parts("/chat/completions", None);
    let key = TestKey::default();
    let fixture = r#"{"model": "gpt", "include_fallback_errors": true, "proxy_server_request": {"arrival_time": 1.0}}"#;
    let input = input_for(fixture, &request, &key, RouteType::Completion);
    let output = step.run(input.data.clone(), &input, None).await.unwrap();
    assert_eq!(
        fake.events(),
        vec![
            Call::Prepare,
            Call::Settings,
            Call::Merge(None),
            Call::Compress,
            Call::Prefetch(String::from("gpt")),
            Call::Hook(false),
        ]
    );
    assert_eq!(text_at(&output.data, "model"), "gpt");
    assert_eq!(text_at(&output.data, "litellm_call_id").len(), 36);
    assert_eq!(output.data.get("include_fallback_errors"), None);
    assert!(output.data.contains_key("metadata"));
    assert_eq!(output.data.get("router_settings_override"), None);
}

#[tokio::test]
async fn step_rejects_a_non_string_model_before_any_port() {
    let fake = Arc::new(Fake::new(behavior()));
    let step = step_for(&fake);
    let request = parts("/chat/completions", None);
    let key = TestKey::default();
    let expected = data_from(r#"{"model": 7}"#);
    let input = call_input(expected.clone(), &request, &key, RouteType::Completion);
    let result = step.run(input.data.clone(), &input, None).await;
    let error = match result {
        Ok(_) => panic!("a non-string model must fail"),
        Err(error) => error,
    };
    assert_eq!(fake.events(), Vec::new());
    assert_eq!(
        error.failure,
        PreCallFailure::Proxy(ProxyFailure {
            message: String::from("'model' must be a string."),
            error_type: String::from("bad_request_error"),
            param: Some(String::from("model")),
            code: 400,
        })
    );
    assert_eq!(error.data, expected);
}

#[tokio::test]
async fn step_failure_returns_the_data_as_it_stood() {
    let script = Behavior {
        hook_failure: Some(boom()),
        ..behavior()
    };
    let fake = Arc::new(Fake::new(script));
    let step = step_for(&fake);
    let request = parts("/chat/completions", Some("call-9"));
    let key = TestKey::default();
    let input = input_for(r#"{"model": "gpt"}"#, &request, &key, RouteType::Completion);
    let result = step.run(input.data.clone(), &input, None).await;
    let error = match result {
        Ok(_) => panic!("the hook failure must surface"),
        Err(error) => error,
    };
    assert_eq!(error.failure, boom());
    assert_eq!(text_at(&error.data, "litellm_call_id"), "call-9");
    assert_eq!(text_at(&error.data, "model"), "gpt");
}

#[tokio::test]
async fn step_writes_the_router_settings_and_weights() {
    let configured =
        r#"{"weights": {"group": {"a": 1.5}}, "model_group_alias": {"gpt": "gpt-alias"}}"#;
    let script = Behavior {
        settings: Some(data_from(configured)),
        ..behavior()
    };
    let fake = Arc::new(Fake::new(script));
    let step = step_for(&fake);
    let request = parts("/chat/completions", None);
    let key = TestKey::default();
    let input = input_for(r#"{"model": "gpt"}"#, &request, &key, RouteType::Completion);
    let output = step.run(input.data.clone(), &input, None).await.unwrap();
    assert_eq!(
        output.data.get("router_settings_override"),
        Some(&JsonValue::Object(data_from(configured)))
    );
    let weights = output.data.get("_router_weights").unwrap();
    let weight = weights.get("group").unwrap().get("a").unwrap();
    assert_eq!(weight.as_f64(), Some(1.5_f64));
    assert_eq!(text_at(&output.data, "model"), "gpt-alias");
    assert!(
        fake.events()
            .contains(&Call::Authorize(String::from("gpt-alias")))
    );
}

#[tokio::test]
async fn step_nulls_invalid_router_weights() {
    let script = Behavior {
        settings: Some(data_from(r#"{"weights": "nope"}"#)),
        ..behavior()
    };
    let fake = Arc::new(Fake::new(script));
    let step = step_for(&fake);
    let request = parts("/chat/completions", None);
    let key = TestKey::default();
    let input = input_for(r#"{"model": "gpt"}"#, &request, &key, RouteType::Completion);
    let output = step.run(input.data.clone(), &input, None).await.unwrap();
    assert_eq!(output.data.get("_router_weights"), Some(&JsonValue::Null));
    assert_eq!(text_at(&output.data, "model"), "gpt");
}

#[tokio::test]
async fn step_skips_router_settings_without_a_router() {
    let script = Behavior {
        settings: Some(data_from(r#"{"model_group_alias": {"gpt": "gpt-alias"}}"#)),
        ..behavior()
    };
    let fake = Arc::new(Fake::new(script));
    let step = step_without_router(&fake);
    let request = parts("/chat/completions", None);
    let key = TestKey::default();
    let input = input_for(r#"{"model": "gpt"}"#, &request, &key, RouteType::Completion);
    let output = step.run(input.data.clone(), &input, None).await.unwrap();
    assert!(!fake.events().contains(&Call::Settings));
    assert_eq!(output.data.get("router_settings_override"), None);
    assert_eq!(text_at(&output.data, "model"), "gpt");
}

#[tokio::test]
async fn step_sets_the_call_id_and_tracer_tags() {
    let fake = Arc::new(Fake::new(behavior()));
    let tracer = Arc::new(TestTracer::default());
    let loggers: Arc<dyn LoggerResolver> = Arc::new(NoLoggers);
    let settings = PreCallSettings::default();
    let step = step_full(&fake, tracer.clone(), loggers, settings);
    let request = parts("/chat/completions", Some("call-1"));
    let key = TestKey::default();
    let input = input_for(r#"{"model": "gpt"}"#, &request, &key, RouteType::Completion);
    let output = step.run(input.data.clone(), &input, None).await.unwrap();
    assert_eq!(text_at(&output.data, "litellm_call_id"), "call-1");
    let tags = tracer.tags();
    assert!(tags.contains(&(String::from("litellm.call_id"), String::from("call-1"))));
    assert!(tags.contains(&(String::from("litellm.requested_model"), String::from("gpt"))));
}

#[tokio::test]
async fn step_resolves_per_request_callbacks() {
    let fake = Arc::new(Fake::new(behavior()));
    let tracer: Arc<dyn Tracer + Send + Sync> = Arc::new(NullTracer);
    let loggers: Arc<dyn LoggerResolver> = Arc::new(TestLoggers);
    let settings = PreCallSettings::default();
    let step = step_full(&fake, tracer, loggers, settings);
    let request = parts("/chat/completions", None);
    let key = TestKey::default();
    let fixture = r#"{"model": "gpt", "callbacks": ["test-logger"]}"#;
    let input = input_for(fixture, &request, &key, RouteType::Completion);
    let output = step.run(input.data.clone(), &input, None).await.unwrap();
    assert_eq!(output.data.get("callbacks"), None);
    let success = output.logging.dynamic_callbacks.success.as_ref().unwrap();
    assert!(success.contains(&String::from("test-logger")));
}

#[tokio::test]
async fn step_merges_stream_usage_only_when_supported() {
    let supported = stream_data(true).await;
    let options = supported.get("stream_options").unwrap();
    assert_eq!(options.get("include_usage"), Some(&JsonValue::Bool(true)));
    assert_eq!(
        supported.get("_litellm_strip_stream_usage"),
        Some(&JsonValue::Bool(true))
    );
    let unsupported = stream_data(false).await;
    assert_eq!(unsupported.get("stream_options"), None);
}

#[tokio::test]
async fn step_merges_guardrails_around_a_rate_limited_model() {
    assert_eq!(
        merges_for(Some("original")).await,
        vec![
            Call::Merge(Some(String::from("original"))),
            Call::Merge(None),
        ]
    );
    assert_eq!(merges_for(None).await, vec![Call::Merge(None)]);
}

#[tokio::test]
async fn step_checks_the_tags_the_hook_added() {
    let checked = tag_events(false).await;
    assert!(checked.contains(&Call::Skip(String::from("/chat/completions"))));
    assert!(checked.contains(&Call::Tags(vec![String::from("added")])));
    let skipped = tag_events(true).await;
    assert!(skipped.contains(&Call::Skip(String::from("/chat/completions"))));
    assert!(!skipped.iter().any(|call| matches!(call, Call::Tags(_))));
}

#[tokio::test]
async fn step_authorizes_stores_only_for_the_responses_routes() {
    let sorted = vec![
        Call::Access(String::from("a")),
        Call::Access(String::from("b")),
    ];
    assert_eq!(stores_for(RouteType::Responses).await, sorted);
    assert_eq!(stores_for(RouteType::ResponsesWebsocket).await, sorted);
    assert_eq!(stores_for(RouteType::Completion).await, Vec::new());
}

#[tokio::test]
async fn step_returns_a_store_access_failure_with_the_data() {
    let script = Behavior {
        access: Some(boom()),
        ..behavior()
    };
    let fake = Arc::new(Fake::new(script));
    let step = step_for(&fake);
    let request = parts("/v1/responses", None);
    let key = TestKey::default();
    let fixture =
        r#"{"model": "gpt", "tools": [{"type": "file_search", "vector_store_ids": ["a"]}]}"#;
    let input = input_for(fixture, &request, &key, RouteType::Responses);
    let result = step.run(input.data.clone(), &input, None).await;
    let error = match result {
        Ok(_) => panic!("the store failure must surface"),
        Err(error) => error,
    };
    assert_eq!(error.failure, boom());
    assert_eq!(text_at(&error.data, "model"), "gpt");
}

#[tokio::test]
async fn step_attaches_retrieval_only_for_get_responses() {
    assert!(
        attach_events(RouteType::GetResponses)
            .await
            .contains(&Call::Attach)
    );
    assert!(
        !attach_events(RouteType::Completion)
            .await
            .contains(&Call::Attach)
    );
}

#[tokio::test]
async fn step_writes_queue_time_to_the_route_bucket() {
    let fake = Arc::new(Fake::new(behavior()));
    let step = step_for(&fake);
    let request = parts("/v1/responses", None);
    let key = TestKey::default();
    let fixture = r#"{"model": "gpt", "proxy_server_request": {"arrival_time": 1.0}}"#;
    let input = input_for(fixture, &request, &key, RouteType::Responses);
    let output = step.run(input.data.clone(), &input, None).await.unwrap();
    let bucket = output.data.get("litellm_metadata").unwrap();
    let queue_time = bucket
        .get("queue_time_seconds")
        .and_then(JsonValue::as_f64)
        .unwrap();
    assert!(queue_time > 1.0_f64);
    assert_eq!(output.data.get("metadata"), None);
}

#[tokio::test]
async fn fallback_returns_the_first_failure_without_fallbacks() {
    let script = Behavior {
        rate_limited_models: vec![String::from("gpt")],
        ..behavior()
    };
    let (events, result) = fallback_run(script, r#"{"model": "gpt"}"#, TestKey::default()).await;
    let error = match result {
        Ok(_) => panic!("the rate limit must surface"),
        Err(error) => error,
    };
    assert_eq!(error.failure, rate_limited());
    assert!(!events.contains(&Call::Fallbacks));
}

#[tokio::test]
async fn fallback_returns_the_failure_when_disabled() {
    let script = fallback_script(&["a"], &["gpt"]);
    let fixture = r#"{"model": "gpt", "disable_fallbacks": true}"#;
    let (events, result) = fallback_run(script, fixture, TestKey::default()).await;
    let error = match result {
        Ok(_) => panic!("the rate limit must surface"),
        Err(error) => error,
    };
    assert_eq!(error.failure, rate_limited());
    assert!(
        !events
            .iter()
            .any(|call| matches!(call, Call::FallbackModels(_, _)))
    );
}

#[tokio::test]
async fn fallback_uses_the_key_router_settings() {
    let mut script = fallback_script(&["key-model"], &["gpt"]);
    script.fallbacks = Some(vec![JsonValue::String(String::from("router-model"))]);
    let mut key = TestKey::default();
    key.router_settings = Some(JsonValue::Object(data_from(
        r#"{"fallbacks": ["key-model"]}"#,
    )));
    let (events, result) = fallback_run(script, r#"{"model": "gpt"}"#, key).await;
    result.unwrap();
    assert!(events.contains(&Call::FallbackModels(
        String::from("gpt"),
        vec![String::from("key-model")],
    )));
    assert!(!events.contains(&Call::Fallbacks));
}

#[tokio::test]
async fn fallback_moves_on_after_a_second_rate_limit() {
    let script = fallback_script(&["a", "b"], &["gpt", "a"]);
    let (events, result) = fallback_run(script, r#"{"model": "gpt"}"#, TestKey::default()).await;
    let output = result.unwrap();
    assert_eq!(text_at(&output.data, "model"), "b");
    assert_eq!(hook_count(&events), 3);
}

#[tokio::test]
async fn fallback_returns_a_successful_fallback() {
    let script = fallback_script(&["a"], &["gpt"]);
    let (events, result) = fallback_run(script, r#"{"model": "gpt"}"#, TestKey::default()).await;
    let output = result.unwrap();
    assert_eq!(text_at(&output.data, "model"), "a");
    assert_eq!(hook_count(&events), 2);
}

#[tokio::test]
async fn fallback_returns_the_original_error_after_the_loop() {
    let script = fallback_script(&["a"], &["gpt", "a"]);
    let (_, result) = fallback_run(script, r#"{"model": "gpt"}"#, TestKey::default()).await;
    let error = match result {
        Ok(_) => panic!("every fallback is rate limited"),
        Err(error) => error,
    };
    assert_eq!(error.failure, rate_limited());
    assert_eq!(text_at(&error.data, "model"), "gpt");
}

#[tokio::test]
async fn fallback_keeps_the_first_tag_set() {
    let mut script = fallback_script(&["a"], &["gpt"]);
    script.skip_budget = false;
    script.tag_on_prepare = Some(String::from("prepared"));
    let (events, result) = fallback_run(script, r#"{"model": "gpt"}"#, TestKey::default()).await;
    let output = result.unwrap();
    assert_eq!(text_at(&output.data, "model"), "a");
    assert!(events.contains(&Call::Tags(vec![String::from("prepared-a")])));
    assert!(!events.contains(&Call::Tags(vec![String::from("prepared-gpt")])));
}

#[tokio::test]
async fn standalone_passes_the_data_through() {
    let settings = PreCallSettings::default();
    let step = PreCallStep::<TestKey>::standalone(settings);
    let request = parts("/chat/completions", None);
    let key = TestKey::default();
    let fixture = r#"{"model": "gpt", "stream": true, "stream_options": null, "messages": "hi"}"#;
    let input = input_for(fixture, &request, &key, RouteType::Completion);
    let output = step.pre_call(input).await.unwrap();
    assert_eq!(text_at(&output.data, "model"), "gpt");
    assert!(output.data.contains_key("litellm_call_id"));
    assert_eq!(output.data.get("stream_options"), Some(&JsonValue::Null));
    assert_eq!(
        output.data.get("messages"),
        data_from(fixture).get("messages")
    );
    assert_eq!(output.logging.model, "gpt");
    assert_eq!(output.logging.stream, Some(true));
    assert!(output.logging.stream_options.is_none());
    assert!(output.logging.messages.is_some());
}

#[tokio::test]
async fn standalone_applies_the_cli_overrides() {
    let mut aliases = BTreeMap::new();
    aliases.insert(String::from("gpt"), String::from("gpt-4o"));
    let pre_call = PreCallSection {
        always_include_stream_usage: None,
        model_alias_map: aliases,
    };
    let overrides = CliOverrides {
        temperature: Some(0.25),
        request_timeout: None,
        max_tokens: None,
        api_base: None,
    };
    let settings = settings_with(overrides, pre_call);
    let step = PreCallStep::<TestKey>::standalone(settings);
    let request = parts("/chat/completions", None);
    let key = TestKey::default();
    let input = input_for(r#"{"model": "gpt"}"#, &request, &key, RouteType::Completion);
    let output = step.pre_call(input).await.unwrap();
    assert_eq!(text_at(&output.data, "model"), "gpt-4o");
    assert_eq!(
        output.data.get("temperature").and_then(JsonValue::as_f64),
        Some(0.25_f64)
    );
}

#[tokio::test]
async fn standalone_runs_without_a_router() {
    let settings = PreCallSettings::default();
    let step = PreCallStep::<TestKey>::standalone(settings);
    let request = parts("/v1/responses", None);
    let key = TestKey::default();
    let fixture =
        r#"{"model": "gpt", "tools": [{"type": "file_search", "vector_store_ids": ["a"]}]}"#;
    let input = input_for(fixture, &request, &key, RouteType::Responses);
    let output = step.pre_call(input).await.unwrap();
    assert_eq!(text_at(&output.data, "model"), "gpt");
    assert_eq!(output.data.get("tools"), data_from(fixture).get("tools"));
    assert_eq!(output.data.get("_router_weights"), None);
}

#[tokio::test]
async fn step_builds_the_logging_state() {
    let fake = Arc::new(Fake::new(behavior()));
    let step = step_for(&fake);
    let request = parts("/chat/completions", None);
    let key = TestKey::default();
    let fixture = r#"{"model": "gpt", "metadata": {"team": "a"}, "stream_options": {"include_usage": true}, "messages": ["hi"]}"#;
    let input = input_for(fixture, &request, &key, RouteType::Completion);
    let output = step.run(input.data.clone(), &input, None).await.unwrap();
    assert_eq!(output.logging.model, "gpt");
    assert!(output.logging.litellm_params.contains_key("metadata"));
    assert!(output.logging.litellm_params.contains_key("api_base"));
    assert!(output.logging.stream_options.is_some());
    assert_eq!(output.logging.stream, Some(false));
    assert_eq!(
        output.logging.messages,
        Some(vec![Json::Text(String::from("hi"))])
    );
}

#[rstest]
#[case("\"\"")]
#[case("0")]
#[case("[]")]
#[case("{}")]
#[tokio::test]
async fn fallback_ignores_a_falsey_disable_flag(#[case] flag: &str) {
    let script = fallback_script(&["a"], &["gpt"]);
    let fixture = format!(r#"{{"model": "gpt", "disable_fallbacks": {flag}}}"#);
    let (_, result) = fallback_run(script, &fixture, TestKey::default()).await;
    result.unwrap();
}

#[tokio::test]
async fn step_splits_the_callback_lists() {
    let fake = Arc::new(Fake::new(behavior()));
    let tracer: Arc<dyn Tracer + Send + Sync> = Arc::new(NullTracer);
    let loggers: Arc<dyn LoggerResolver> = Arc::new(TestLoggers);
    let step = step_full(&fake, tracer, loggers, PreCallSettings::default());
    let request = parts("/chat/completions", None);
    let key = TestKey::default();
    let fixture = r#"{"model": "gpt", "success_callback": ["dynamodb", "langfuse"], "failure_callback": ["langfuse2"], "callbacks": ["test-logger", "test-logger"]}"#;
    let input = input_for(fixture, &request, &key, RouteType::Completion);
    let output = step.run(input.data.clone(), &input, None).await.unwrap();
    let dynamic = &output.logging.dynamic_callbacks;
    assert_eq!(
        dynamic.success,
        Some(vec![String::from("langfuse"), String::from("test-logger")])
    );
    assert_eq!(
        dynamic.async_success,
        Some(vec![String::from("dynamodb"), String::from("test-logger")])
    );
    assert_eq!(
        dynamic.failure,
        Some(vec![String::from("langfuse2"), String::from("test-logger")])
    );
    assert_eq!(
        dynamic.async_failure,
        Some(vec![String::from("test-logger")])
    );
    assert_eq!(dynamic.input, Some(vec![String::from("test-logger")]));
}

#[tokio::test]
async fn step_uses_the_image_model_for_image_edits() {
    let fake = Arc::new(Fake::new(behavior()));
    let tracer: Arc<dyn Tracer + Send + Sync> = Arc::new(NullTracer);
    let loggers: Arc<dyn LoggerResolver> = Arc::new(NoLoggers);
    let mut settings = PreCallSettings::default();
    settings.model_defaults.image_generation_model = Some(String::from("img-model"));
    let step = step_full(&fake, tracer, loggers, settings);
    let request = parts("/v1/images/edits", None);
    let key = TestKey::default();
    let input = input_for(r#"{"model": "gpt"}"#, &request, &key, RouteType::ImageEdit);
    let output = step.run(input.data.clone(), &input, None).await.unwrap();
    assert_eq!(text_at(&output.data, "model"), "img-model");
}

#[tokio::test]
async fn standalone_router_settings_answer_none() {
    let fake = Arc::new(Fake::new(behavior()));
    let tracer: Arc<dyn Tracer + Send + Sync> = Arc::new(NullTracer);
    let mut built = ports(&fake, tracer);
    built.router_settings = Arc::new(NoRouterSettings);
    let loggers: Arc<dyn LoggerResolver> = Arc::new(NoLoggers);
    let step = PreCallStep::new(built, PreCallSettings::default(), loggers);
    let request = parts("/chat/completions", None);
    let key = TestKey::default();
    let input = input_for(r#"{"model": "gpt"}"#, &request, &key, RouteType::Completion);
    let output = step.run(input.data.clone(), &input, None).await.unwrap();
    assert_eq!(output.data.get("router_settings_override"), None);
}

#[tokio::test]
async fn standalone_allows_the_tag_budgets() {
    let budgets = AllowAllBudgets;
    let model = RequestModel::Absent;
    let skips = budgets.skips_budget("/chat/completions", &model).await;
    assert!(skips);
    let key = TestKey::default();
    let tags = [String::from("tag")];
    budgets.check_tags(&tags, &key).await.unwrap();
}
