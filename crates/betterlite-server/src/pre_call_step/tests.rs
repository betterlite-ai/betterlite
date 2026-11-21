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

use super::standalone::{AllowAllBudgets, NoRouterSettings};
use super::{PreCall as _, PreCallInput, PreCallSettings, PreCallStep};
use crate::pre_call::failure::{PreCallFailure, ProxyFailure};
use crate::pre_call::key::PreCallKey;
use crate::pre_call::ports::{
    CompressionArming, GuardrailMerge, PreCallHooks, PreCallPorts, RequestPreparer,
    RetrievalPipelines, RouterSettingsSource, RouterView, StreamOptionsCheck, TagBudgets,
    VectorStoreAccess,
};
use crate::pre_call::rewrites::{CliOverrides, RequestModel};
use crate::pre_call::route_type::RouteType;

// the fallback-tests check selects names beginning pre_call_step::tests::fallback_
#[path = "tests/fallbacks.rs"]
mod fallback_tests;
mod redaction;
mod support;

use support::{Fake, NoLoggers, TestLoggers, TestTracer};

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
    fail_models: Vec<String>,
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
        fail_models: Vec::new(),
        authorize: None,
        access: None,
        tag_on_prepare: None,
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
    if let Some(id) = call_id {
        builder = builder.header("x-litellm-call-id", id);
    }
    let request = builder.body(()).unwrap();
    request.into_parts().0
}

fn text_at<'data>(data: &'data JsonMap, field: &str) -> &'data str {
    data.get(field)
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
}

fn input_for<'input>(
    fixture: &str,
    request: &'input Parts,
    key: &'input TestKey,
    route_type: RouteType,
) -> PreCallInput<'input, TestKey> {
    PreCallInput {
        data: data_from(fixture),
        request,
        key,
        route_type,
        version: None,
        model: None,
        skip_guardrails: false,
    }
}

fn ports(fake: &Arc<Fake>, tracer: Arc<dyn Tracer + Send + Sync>) -> PreCallPorts<TestKey> {
    let preparer: Arc<dyn RequestPreparer<TestKey>> = Arc::<Fake>::clone(fake);
    let vector_stores: Arc<dyn VectorStoreAccess<TestKey>> = Arc::<Fake>::clone(fake);
    let router_settings: Arc<dyn RouterSettingsSource<TestKey>> = Arc::<Fake>::clone(fake);
    let router: Arc<dyn RouterView<TestKey>> = Arc::<Fake>::clone(fake);
    let stream_options: Arc<dyn StreamOptionsCheck> = Arc::<Fake>::clone(fake);
    let guardrail_merge: Arc<dyn GuardrailMerge> = Arc::<Fake>::clone(fake);
    let compression: Arc<dyn CompressionArming> = Arc::<Fake>::clone(fake);
    let hooks: Arc<dyn PreCallHooks<TestKey>> = Arc::<Fake>::clone(fake);
    let tag_budgets: Arc<dyn TagBudgets<TestKey>> = Arc::<Fake>::clone(fake);
    let retrieval: Arc<dyn RetrievalPipelines<TestKey>> = Arc::<Fake>::clone(fake);
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

async fn events_after(
    script: Behavior,
    path: &str,
    fixture: &str,
    route_type: RouteType,
    rate_limited_model: Option<&str>,
) -> Vec<Call> {
    let fake = Arc::new(Fake::new(script));
    let step = step_for(&fake);
    let request = parts(path, None);
    let key = TestKey::default();
    let input = input_for(fixture, &request, &key, route_type);
    step.run(input.data.clone(), &input, rate_limited_model)
        .await
        .unwrap();
    fake.events()
}

async fn stores_for(route_type: RouteType) -> Vec<Call> {
    let fixture =
        r#"{"model": "gpt", "tools": [{"type": "file_search", "vector_store_ids": ["b", "a"]}]}"#;
    let events = events_after(behavior(), "/v1/responses", fixture, route_type, None).await;
    access_calls(&events)
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
    let fixture = r#"{"model": "gpt"}"#;
    let events = events_after(
        behavior(),
        "/chat/completions",
        fixture,
        RouteType::Completion,
        rate_limited_model,
    )
    .await;
    merge_calls(&events)
}

async fn tag_events(skip_budget: bool) -> Vec<Call> {
    let script = Behavior {
        skip_budget,
        hook_tags: vec![String::from("added")],
        ..behavior()
    };
    let fixture = r#"{"model": "gpt", "metadata": {"tags": ["first"]}}"#;
    events_after(
        script,
        "/chat/completions",
        fixture,
        RouteType::Completion,
        None,
    )
    .await
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
    let snapshot = output
        .data
        .get("proxy_server_request")
        .and_then(|held| held.get("body"));
    assert_eq!(
        snapshot
            .and_then(|body| body.get("model"))
            .and_then(JsonValue::as_str),
        Some("gpt")
    );
}

#[tokio::test]
async fn step_rejects_a_non_string_model_before_any_port() {
    let fake = Arc::new(Fake::new(behavior()));
    let step = step_for(&fake);
    let request = parts("/chat/completions", None);
    let key = TestKey::default();
    let input = input_for(r#"{"model": 7}"#, &request, &key, RouteType::Completion);
    let expected = input.data.clone();
    let result = step.run(expected.clone(), &input, None).await;
    let Err(error) = result else {
        panic!("a non-string model must fail")
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
async fn step_resolves_a_null_model_to_the_default() {
    let fake = Arc::new(Fake::new(behavior()));
    let tracer: Arc<dyn Tracer + Send + Sync> = Arc::new(NullTracer);
    let loggers: Arc<dyn LoggerResolver> = Arc::new(NoLoggers);
    let mut settings = PreCallSettings::default();
    settings.model_defaults.completion_model = Some(String::from("default-model"));
    let step = step_full(&fake, tracer, loggers, settings);
    let request = parts("/chat/completions", None);
    let key = TestKey::default();
    let input = input_for(r#"{"model": null}"#, &request, &key, RouteType::Completion);
    let output = step.run(input.data.clone(), &input, None).await.unwrap();
    assert_eq!(text_at(&output.data, "model"), "default-model");
    assert!(fake.events().contains(&Call::Prepare));
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
    let Err(error) = result else {
        panic!("the hook failure must surface")
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
    let step = step_full(&fake, Arc::<TestTracer>::clone(&tracer), loggers, settings);
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
    let Err(error) = result else {
        panic!("the store failure must surface")
    };
    assert_eq!(error.failure, boom());
    assert_eq!(text_at(&error.data, "model"), "gpt");
}

#[tokio::test]
async fn step_attaches_retrieval_only_for_get_responses() {
    let fixture = r#"{"model": "gpt"}"#;
    let attached = events_after(
        behavior(),
        "/v1/responses/resp_1",
        fixture,
        RouteType::GetResponses,
        None,
    )
    .await;
    let skipped = events_after(
        behavior(),
        "/v1/responses/resp_1",
        fixture,
        RouteType::Completion,
        None,
    )
    .await;
    assert!(attached.contains(&Call::Attach));
    assert!(!skipped.contains(&Call::Attach));
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
        temperature: Some(0.25_f64),
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
    assert_eq!(output.data.get("callbacks"), None);
    assert_eq!(output.data.get("success_callback"), None);
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
    let skips = TagBudgets::<TestKey>::skips_budget(&budgets, "/chat/completions", &model).await;
    assert!(skips);
    let key = TestKey::default();
    let tags = [String::from("tag")];
    budgets.check_tags(&tags, &key).await.unwrap();
}

#[tokio::test]
async fn step_rejects_a_url_valued_model_argument() {
    let fake = Arc::new(Fake::new(behavior()));
    let step = step_for(&fake);
    let request = parts("/v1/images/edits", None);
    let key = TestKey::default();
    let mut input = input_for(r#"{"model": "gpt"}"#, &request, &key, RouteType::ImageEdit);
    input.model = Some("https://example.com/model");
    let data = input.data.clone();
    let result = step.run(data.clone(), &input, None).await;
    let Err(error) = result else {
        panic!("a URL-valued model argument must fail")
    };
    let (status, detail) = match error.failure {
        PreCallFailure::Http { status, detail } => (status, detail),
        failure @ (PreCallFailure::Proxy(_) | PreCallFailure::RateLimited(_)) => {
            panic!("expected an HTTP failure, got {failure:?}")
        }
    };
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("param"),
        Some(&JsonValue::String(String::from("model")))
    );
    assert_eq!(error.data, data);
}
