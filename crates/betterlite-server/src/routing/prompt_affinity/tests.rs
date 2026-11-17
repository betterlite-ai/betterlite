#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::prompt_affinity as section;
use betterlite_config::types::Config;
use betterlite_core::message_utilities::prompt_cache_checks::PromptCacheSettings;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_prompts::cache_accounting::dialect::ModelFacts;
use betterlite_prompts::cache_defaults::CacheFacts;
use betterlite_router::prompt_affinity::service::{InjectionDefaults, TokenGate};
use betterlite_router::router_core::services::RouteServices;
use indexmap::IndexMap;

use crate::routing::prompt_affinity::settings;
use crate::routing::router_core::RouterServices;

use super::{DualCachePins, PinCache, PromptAffinityInputs};

fn configured(prompt_caching: bool, maximum_base64_length: Option<&str>) -> section::Settings {
    section::Settings {
        prompt_caching,
        maximum_base64_length: maximum_base64_length.map(str::to_owned),
    }
}

fn pin(model_id: &str) -> Json {
    Json::Object(IndexMap::from([(
        "model_id".to_owned(),
        Json::Text(model_id.to_owned()),
    )]))
}

fn clock_at(handle: Arc<Mutex<f64>>) -> Arc<dyn Fn() -> f64 + Send + Sync> {
    Arc::new(move || *handle.lock().unwrap())
}

fn cache_at(now: Arc<Mutex<f64>>) -> Arc<DualCache<LocalMemoryOps>> {
    let memory = LocalMemoryOps::new(1024, clock_at(now)).unwrap();
    Arc::new(DualCache::new(
        Arc::new(memory),
        None,
        DualCacheSettings {
            default_in_memory_ttl: None,
            default_redis_ttl: None,
            batch_cache_expiry: None,
            max_redis_batch_cache_size: 1000,
            redis_delete_chunk_size: 1000,
        },
    ))
}

fn message_uri(payload: &str) -> Json {
    let source = format!(
        r#"{{"role":"user","content":[{{"type":"image_url","image_url":{{"url":"data:image/png;base64,{payload}"}}}}]}}"#
    );
    serde_json::from_str(&source).unwrap()
}

#[test]
fn the_check_is_off_unless_the_config_names_it() {
    assert!(!settings(&configured(false, None)).unwrap().enabled);
    assert!(settings(&configured(true, None)).unwrap().enabled);
}

#[test]
fn the_configured_base64_limit_decides_which_payloads_shrink() {
    let payload = "A".repeat(12);
    let message = message_uri(&payload);
    let default = settings(&configured(true, None)).unwrap().truncator;
    let short = settings(&configured(true, Some("8"))).unwrap().truncator;
    assert_eq!(default.truncate_messages(&message).unwrap(), message);
    assert_ne!(short.truncate_messages(&message).unwrap(), message);
}

#[test]
fn a_malformed_base64_limit_is_an_error() {
    assert!(settings(&configured(true, Some("many"))).is_err());
}

#[tokio::test]
async fn a_pin_is_readable_until_its_ttl_expires() {
    let now = Arc::new(Mutex::new(0.0_f64));
    let pins: Arc<dyn PinCache> = Arc::new(DualCachePins {
        cache: cache_at(Arc::clone(&now)),
    });
    let key = "deployment:abc:prompt_caching".to_owned();
    pins.store(&key, &pin("dep-a")).await;
    let keys = vec![key];
    assert_eq!(pins.read(&keys).await, [Some(pin("dep-a"))]);
    *now.lock().unwrap() = 299.0_f64;
    assert_eq!(pins.read(&keys).await, [Some(pin("dep-a"))]);
    *now.lock().unwrap() = 301.0_f64;
    assert_eq!(pins.read(&keys).await, [None]);
}

const MESSAGES: &str = r#"[{"role":"system","content":"be brief"},{"role":"user","content":"why is the sky blue","cache_control":{"type":"ephemeral"}}]"#;

const PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.0,
    "completionStartTime": 1704067201.0,
    "response_time": 1.0,
    "model_map_information": {},
    "model": "claude-min",
    "model_id": "dep-a",
    "api_base": "https://api.anthropic.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {},
    "messages": [{"role": "system", "content": "be brief"}, {"role": "user", "content": "why is the sky blue", "cache_control": {"type": "ephemeral"}}]
}"#;

struct AllowAll;

#[async_trait]
impl TokenGate for AllowAll {
    async fn meets_minimum(&self, _model: &str, _messages: &[Json], _minimum: Option<u64>) -> bool {
        true
    }
}

struct NoFacts;

impl ModelFacts for NoFacts {
    fn cache_breakpoint_flag(&self, _model: &str) -> Option<bool> {
        None
    }

    fn provider(&self, _model: &str) -> Option<String> {
        None
    }
}

impl CacheFacts for NoFacts {
    fn supports_prompt_caching(&self, _model: &str, _provider: &str) -> bool {
        false
    }
}

fn inputs(cache: Arc<DualCache<LocalMemoryOps>>) -> PromptAffinityInputs<LocalMemoryOps> {
    PromptAffinityInputs {
        cache,
        gate: Arc::new(AllowAll),
        facts: Arc::new(NoFacts),
        catalog: Arc::new(
            ModelCatalog::parse(b"{}", UnknownFields::Warn).expect("an empty catalog parses"),
        ),
        settings: PromptCacheSettings::default(),
        defaults: InjectionDefaults::default(),
    }
}

fn loaded(checks: &[&str], limit: Option<&str>) -> Config {
    let checks = checks.join(", ");
    let source =
        format!("model_list: []\nrouter_settings:\n  optional_pre_call_checks: [{checks}]\n");
    let mut lookup = BTreeMap::new();
    if let Some(limit) = limit {
        lookup.insert("MAX_BASE64_LENGTH_FOR_LOGGING".to_owned(), limit.to_owned());
    }
    ConfigLoader { lookup }
        .load(&source)
        .expect("the test config loads")
}

fn event(payload: StandardLoggingPayload) -> Arc<LoggerEvent> {
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Null),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    })
}

fn payload() -> StandardLoggingPayload {
    serde_json::from_str(PAYLOAD).expect("the payload fixture parses")
}

fn clock() -> Arc<Mutex<f64>> {
    Arc::new(Mutex::new(0.0_f64))
}

async fn pinned_after(services: &RouterServices) -> Option<String> {
    let messages: Vec<Json> = serde_json::from_str(MESSAGES).expect("the messages fixture parses");
    services
        .prompt_affinity()
        .expect("the check is registered")
        .pin_for(Some(&messages), None)
        .await
        .expect("the pin cache answers")
}

#[tokio::test]
async fn a_success_event_pins_the_deployment_the_next_request_reuses() {
    let services = RouterServices::from_config(
        &loaded(&["prompt_caching"], None),
        inputs(cache_at(clock())),
    )
    .expect("the router builds");
    assert_eq!(services.callback_name(), "PromptCachingDeploymentCheck");
    services
        .async_log_success_event(event(payload()))
        .await
        .expect("the success event is logged");
    assert_eq!(pinned_after(&services).await.as_deref(), Some("dep-a"));
}

// a standard_logging_object without a model id names no deployment to pin
#[tokio::test]
async fn a_success_event_without_a_model_id_never_pins() {
    let services = RouterServices::from_config(
        &loaded(&["prompt_caching"], None),
        inputs(cache_at(clock())),
    )
    .expect("the router builds");
    let mut payload = payload();
    payload.model_id = None;
    services
        .async_log_success_event(event(payload))
        .await
        .expect("the success event is logged");
    assert!(pinned_after(&services).await.is_none());
}

// a standard_logging_object whose messages are not a list is skipped
#[tokio::test]
async fn a_success_event_without_a_message_list_never_pins() {
    let services = RouterServices::from_config(
        &loaded(&["prompt_caching"], None),
        inputs(cache_at(clock())),
    )
    .expect("the router builds");
    let mut payload = payload();
    payload.messages = Some(Json::Text("the prompt".to_owned()));
    services
        .async_log_success_event(event(payload))
        .await
        .expect("the success event is logged");
    assert!(pinned_after(&services).await.is_none());
}

// without the config entry the router registers no check, and a success event leaves no pin
#[tokio::test]
async fn a_router_without_the_check_ignores_successes() {
    let services = RouterServices::from_config(&loaded(&[], None), inputs(cache_at(clock())))
        .expect("the router builds");
    assert!(services.prompt_affinity().is_none());
    services
        .async_log_success_event(event(payload()))
        .await
        .expect("an unregistered check ignores the event");
}

// MAX_BASE64_LENGTH_FOR_LOGGING is read once at setup, and a value that is not a length stops it
#[tokio::test]
async fn a_malformed_base64_limit_fails_the_router_setup() {
    assert!(
        RouterServices::from_config(
            &loaded(&["prompt_caching"], Some("many")),
            inputs(cache_at(clock()))
        )
        .is_err()
    );
}
