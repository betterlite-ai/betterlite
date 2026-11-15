#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json as DecisionJson;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::{ProviderIdentity, ResolutionContext};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde_json::Number;
use thiserror::Error;

use super::config::GenerateContentProviderConfig;
use super::config::GenerateContentProviderConfigs;
use super::config::GenerateContentRequest;
use super::config::provider_config_for;
use super::error::GenAiSetupError;
use super::logging::mark_async_entrypoint;
use super::merge::merge_native_request_fields;
use super::params::VertexCredentials;
use super::setup::SetupRequest;
use super::setup::setup_generate_content_call;

const FIELDS: [&str; 4] = ["safetySettings", "toolConfig", "cachedContent", "labels"];

#[derive(Debug, Error)]
#[error("the fixture config refuses to transform {model}")]
struct FixtureError {
    model: String,
}

#[derive(Debug, PartialEq, Eq)]
struct TransformedRequest {
    model: String,
    tools: Option<Json>,
    generation_config: Object,
    system_instruction: Option<Json>,
}

struct FixtureConfig {
    fields: &'static [&'static str],
    fail: bool,
    mapped: Mutex<Vec<Object>>,
    transformed: Mutex<Vec<TransformedRequest>>,
}

impl FixtureConfig {
    fn for_fields(fields: &'static [&'static str]) -> Self {
        Self {
            fields,
            fail: false,
            mapped: Mutex::new(Vec::new()),
            transformed: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            fields: &FIELDS,
            fail: true,
            mapped: Mutex::new(Vec::new()),
            transformed: Mutex::new(Vec::new()),
        }
    }
}

impl GenerateContentProviderConfig for FixtureConfig {
    type Error = FixtureError;

    fn map_optional_params(&self, config: &Object, _model: &str) -> Object {
        self.mapped
            .lock()
            .expect("the fixture lock is not poisoned")
            .push(config.clone());
        config.clone()
    }

    fn transform_request(&self, request: &GenerateContentRequest<'_>) -> Result<Json, Self::Error> {
        self.transformed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(TransformedRequest {
                model: request.model.to_owned(),
                tools: request.tools.cloned(),
                generation_config: request.generation_config.clone(),
                system_instruction: request.system_instruction.cloned(),
            });
        if self.fail {
            return Err(FixtureError {
                model: request.model.to_owned(),
            });
        }
        let mut body = Object::new();
        body.insert("fixture".to_owned(), Json::Bool(true));
        body.insert("model".to_owned(), Json::Text(request.model.to_owned()));
        body.insert("contents".to_owned(), request.contents.clone());
        body.insert(
            "generationConfig".to_owned(),
            Json::Object(request.generation_config.clone()),
        );
        if let Some(tools) = request.tools {
            body.insert("tools".to_owned(), tools.clone());
        }
        if let Some(instruction) = request.system_instruction {
            body.insert("systemInstruction".to_owned(), instruction.clone());
        }
        Ok(Json::Object(body))
    }

    fn request_top_level_fields(&self) -> &'static [&'static str] {
        self.fields
    }
}

fn is_partner(model: &str) -> bool {
    model.starts_with("claude")
}

fn object(entries: &[(&str, Json)]) -> Object {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn decision_object(entries: &[(&str, DecisionJson)]) -> IndexMap<String, DecisionJson> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn provider_configs<'configs>(
    gemini: &'configs FixtureConfig,
    vertex: &'configs FixtureConfig,
) -> GenerateContentProviderConfigs<'configs, FixtureConfig> {
    GenerateContentProviderConfigs {
        gemini,
        vertex,
        is_vertex_partner_model: is_partner,
    }
}

fn setup_request<'request>(
    model: &'request str,
    contents: &'request Json,
    config: Option<&'request Object>,
    tools: Option<&'request Json>,
    kwargs: &'request Object,
) -> SetupRequest<'request> {
    SetupRequest {
        model,
        contents,
        config,
        custom_llm_provider: None,
        tools,
        kwargs,
    }
}

struct Fixture {
    catalog: Arc<ModelCatalog>,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Fixture {
    fn new() -> Self {
        let mut bundled = BundledCatalog::default();
        Self {
            catalog: bundled.get(),
            names: BTreeSet::new(),
            environment: BTreeMap::new(),
        }
    }

    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: self.catalog.as_ref(),
            known_model_names: &self.names,
            custom_providers: &self.names,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
}

fn logging() -> LoggingState {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "fixture-model".to_owned(),
            call_type: "generate_content".to_owned(),
            start_time: moment(),
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: Some(false),
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles"),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds")
}

#[test]
fn provider_dispatch_selects_gemini_config_for_the_gemini_provider() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let contents = text("hello");
    let kwargs = Object::new();
    let request = setup_request("gemini/gemini-2.5-flash", &contents, None, None, &kwargs);
    let mut state = logging();
    let result = setup_generate_content_call(&request, &configs, Some(&mut state), &resolution)
        .expect("the gemini setup succeeds");
    assert_eq!(result.model, "gemini-2.5-flash");
    assert_eq!(result.custom_llm_provider, "gemini");
    assert!(result.provider_config.is_some());
    assert_eq!(
        gemini
            .mapped
            .lock()
            .expect("the fixture lock is not poisoned")
            .len(),
        1
    );
    assert!(
        vertex
            .mapped
            .lock()
            .expect("the fixture lock is not poisoned")
            .is_empty()
    );
}

#[test]
fn provider_dispatch_selects_vertex_config_for_a_native_vertex_model() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let contents = text("hello");
    let kwargs = Object::new();
    let request = setup_request("vertex_ai/gemini-2.5-flash", &contents, None, None, &kwargs);
    let mut state = logging();
    let result = setup_generate_content_call(&request, &configs, Some(&mut state), &resolution)
        .expect("the vertex setup succeeds");
    assert_eq!(result.model, "gemini-2.5-flash");
    assert_eq!(result.custom_llm_provider, "vertex_ai");
    assert!(result.provider_config.is_some());
    assert_eq!(
        vertex
            .mapped
            .lock()
            .expect("the fixture lock is not poisoned")
            .len(),
        1
    );
    assert!(
        gemini
            .mapped
            .lock()
            .expect("the fixture lock is not poisoned")
            .is_empty()
    );
}

#[test]
fn provider_dispatch_falls_back_to_the_adapter_for_vertex_partner_models() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let contents = text("hello");
    let config = object(&[("temperature", text("caller-value"))]);
    let kwargs = Object::new();
    let request = setup_request(
        "vertex_ai/claude-sonnet-4",
        &contents,
        Some(&config),
        None,
        &kwargs,
    );
    let mut state = logging();
    let result = setup_generate_content_call(&request, &configs, Some(&mut state), &resolution)
        .expect("the partner fallback succeeds");
    assert_eq!(result.model, "claude-sonnet-4");
    assert_eq!(result.custom_llm_provider, "vertex_ai");
    assert!(result.provider_config.is_none());
    assert_eq!(result.request_body, Json::Object(Object::new()));
    assert_eq!(result.config_dict, config);
    assert!(result.native_request_fields.is_empty());
    assert!(
        gemini
            .mapped
            .lock()
            .expect("the fixture lock is not poisoned")
            .is_empty()
    );
    assert!(
        vertex
            .transformed
            .lock()
            .expect("the fixture lock is not poisoned")
            .is_empty()
    );
}

#[test]
fn provider_dispatch_falls_back_to_the_adapter_for_other_providers() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let contents = text("hello");
    let kwargs = Object::new();
    let request = setup_request("anthropic/claude-sonnet-4", &contents, None, None, &kwargs);
    let mut state = logging();
    let result = setup_generate_content_call(&request, &configs, Some(&mut state), &resolution)
        .expect("the adapter fallback succeeds");
    assert_eq!(result.model, "claude-sonnet-4");
    assert_eq!(result.custom_llm_provider, "anthropic");
    assert!(result.provider_config.is_none());
    assert_eq!(result.request_body, Json::Object(Object::new()));
}

#[test]
fn provider_dispatch_falls_back_for_an_unknown_provider_slug() {
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let unknown = ProviderIdentity::Custom("custom-provider".to_owned());
    assert!(provider_config_for("gemini-2.5-flash", &unknown, &configs).is_none());
    let beta = ProviderIdentity::Known(ProviderId::VertexAiBeta);
    assert!(provider_config_for("gemini-2.5-flash", &beta, &configs).is_none());
}

fn native_kwargs() -> Object {
    object(&[
        (
            "safetySettings",
            Json::List(vec![Json::Object(object(&[(
                "category",
                text("HARM_CATEGORY_HATE_SPEECH"),
            )]))]),
        ),
        ("labels", Json::Object(object(&[("team", text("x"))]))),
        ("unrelated", text("dropped")),
    ])
}

#[test]
fn native_fields_extracts_only_the_present_listed_fields() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let contents = text("hello");
    let kwargs = native_kwargs();
    let request = setup_request("gemini/gemini-2.5-flash", &contents, None, None, &kwargs);
    let mut state = logging();
    let result = setup_generate_content_call(&request, &configs, Some(&mut state), &resolution)
        .expect("the gemini setup succeeds");
    assert_eq!(
        result.native_request_fields,
        object(&[
            (
                "safetySettings",
                Json::List(vec![Json::Object(object(&[(
                    "category",
                    text("HARM_CATEGORY_HATE_SPEECH"),
                )]))]),
            ),
            ("labels", Json::Object(object(&[("team", text("x"))]))),
        ])
    );
}

#[test]
fn native_fields_follow_the_config_field_list() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&["labels", "customField"]);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let contents = text("hello");
    let kwargs = object(&[
        ("safetySettings", text("not listed by this config")),
        ("labels", Json::Object(object(&[("team", text("x"))]))),
        ("customField", Json::Null),
    ]);
    let request = setup_request("gemini/gemini-2.5-flash", &contents, None, None, &kwargs);
    let mut state = logging();
    let result = setup_generate_content_call(&request, &configs, Some(&mut state), &resolution)
        .expect("the gemini setup succeeds");
    assert_eq!(
        result.native_request_fields,
        object(&[
            ("labels", Json::Object(object(&[("team", text("x"))]))),
            ("customField", Json::Null),
        ])
    );
}

#[test]
fn provider_dispatch_keeps_the_params_the_handlers_read() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let contents = text("hello");
    let kwargs = object(&[
        ("max_retries", Json::Number(Number::from(3_i64))),
        ("vertex_project", text("project-1")),
        ("vertex_location", text("us-central1")),
        ("vertex_credentials", text("creds.json")),
        ("ssl_verify", Json::Bool(false)),
        ("tpm", Json::Number(Number::from(7_i64))),
    ]);
    let request = setup_request("vertex_ai/gemini-2.5-flash", &contents, None, None, &kwargs);
    let mut state = logging();
    let result = setup_generate_content_call(&request, &configs, Some(&mut state), &resolution)
        .expect("the vertex setup succeeds");
    let params = &result.litellm_params;
    assert_eq!(params.max_retries, Some(3));
    assert_eq!(params.vertex_project.as_deref(), Some("project-1"));
    assert_eq!(params.vertex_location.as_deref(), Some("us-central1"));
    assert_eq!(
        params.vertex_credentials,
        Some(VertexCredentials::Path("creds.json".to_owned()))
    );
    assert_eq!(params.ssl_verify, Some(Json::Bool(false)));
    assert_eq!(
        params.extra.get("tpm"),
        Some(&Json::Number(Number::from(7_i64)))
    );
    assert!(!params.extra.contains_key("vertex_project"));
    assert!(!params.extra.contains_key("max_retries"));
}

#[test]
fn provider_dispatch_keeps_the_endpoint_and_account_params() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let contents = text("hello");
    let account = object(&[
        ("type", text("service_account")),
        ("project_id", text("p1")),
    ]);
    let kwargs = object(&[
        ("api_base", text("https://example.test")),
        ("api_key", text("key-1")),
        ("api_version", text("v1beta")),
        ("organization", text("org-1")),
        ("vertex_credentials", Json::Object(account.clone())),
    ]);
    let request = setup_request("vertex_ai/gemini-2.5-flash", &contents, None, None, &kwargs);
    let mut state = logging();
    let result = setup_generate_content_call(&request, &configs, Some(&mut state), &resolution)
        .expect("the vertex setup succeeds");
    let params = &result.litellm_params;
    assert_eq!(params.api_base.as_deref(), Some("https://example.test"));
    let key = params.api_key.as_ref().expect("the api key is kept");
    assert_eq!(key.expose_secret(), "key-1");
    assert_eq!(params.api_version.as_deref(), Some("v1beta"));
    assert_eq!(params.organization.as_deref(), Some("org-1"));
    assert_eq!(
        params.vertex_credentials,
        Some(VertexCredentials::ServiceAccount(account))
    );
    assert!(params.extra.is_empty());
}

#[test]
fn merge_lets_an_explicit_extra_body_value_win() {
    let native = object(&[
        ("labels", text("native")),
        ("cachedContent", text("cached")),
    ]);
    let extra = object(&[("labels", text("caller")), ("extra", text("kept"))]);
    let merged =
        merge_native_request_fields(&native, Some(&extra)).expect("the merge returns a body");
    assert_eq!(
        merged,
        object(&[
            ("labels", text("caller")),
            ("cachedContent", text("cached")),
            ("extra", text("kept")),
        ])
    );
}

#[test]
fn merge_keeps_native_fields_when_extra_body_is_empty() {
    let native = object(&[("labels", text("native"))]);
    let extra = Object::new();
    let merged =
        merge_native_request_fields(&native, Some(&extra)).expect("the merge keeps the fields");
    assert_eq!(merged, native);
}

#[test]
fn merge_returns_the_extra_body_when_there_are_no_native_fields() {
    let native = Object::new();
    let extra = object(&[("extra", text("kept"))]);
    let merged = merge_native_request_fields(&native, Some(&extra))
        .expect("a caller body survives without native fields");
    assert_eq!(merged, extra);
}

#[test]
fn merge_preserves_an_empty_extra_body() {
    let native = Object::new();
    let extra = Object::new();
    let merged = merge_native_request_fields(&native, Some(&extra))
        .expect("an explicit empty body survives");
    assert!(merged.is_empty());
}

#[test]
fn merge_returns_none_only_without_fields_and_extra_body() {
    assert_eq!(merge_native_request_fields(&Object::new(), None), None);
}

#[test]
fn mock_string_response_raises_when_stream_is_defaulted() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let contents = text("hello");
    let kwargs = object(&[("mock_response", text("hi"))]);
    let request = setup_request("gemini/gemini-2.5-flash", &contents, None, None, &kwargs);
    let mut state = logging();
    let error = setup_generate_content_call(&request, &configs, Some(&mut state), &resolution)
        .err()
        .expect("a string mock response is rejected");
    assert!(matches!(
        error,
        GenAiSetupError::MockResponseHandledByCaller
    ));
}

#[test]
fn mock_string_response_proceeds_when_stream_is_true() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let contents = text("hello");
    let kwargs = object(&[("mock_response", text("hi")), ("stream", Json::Bool(true))]);
    let request = setup_request("gemini/gemini-2.5-flash", &contents, None, None, &kwargs);
    let mut state = logging();
    setup_generate_content_call(&request, &configs, Some(&mut state), &resolution)
        .expect("a streaming call skips the mock guard");
    assert_eq!(
        gemini
            .mapped
            .lock()
            .expect("the fixture lock is not poisoned")
            .len(),
        1
    );
}

#[test]
fn mock_response_of_another_type_is_not_rejected() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let contents = text("hello");
    let kwargs = object(&[(
        "mock_response",
        Json::Object(object(&[("text", text("hi"))])),
    )]);
    let request = setup_request("gemini/gemini-2.5-flash", &contents, None, None, &kwargs);
    let mut state = logging();
    setup_generate_content_call(&request, &configs, Some(&mut state), &resolution)
        .expect("only a string mock response is rejected");
}

#[test]
fn logging_is_required_for_the_native_path() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let contents = text("hello");
    let kwargs = Object::new();
    let request = setup_request("gemini/gemini-2.5-flash", &contents, None, None, &kwargs);
    let error = setup_generate_content_call(&request, &configs, None, &resolution)
        .err()
        .expect("the native path requires a logging object");
    assert!(matches!(error, GenAiSetupError::LoggingRequired));
    assert_eq!(
        gemini
            .transformed
            .lock()
            .expect("the fixture lock is not poisoned")
            .len(),
        1
    );
}

#[test]
fn logging_is_required_for_the_adapter_path() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let contents = text("hello");
    let kwargs = Object::new();
    let request = setup_request("vertex_ai/claude-sonnet-4", &contents, None, None, &kwargs);
    let error = setup_generate_content_call(&request, &configs, None, &resolution)
        .err()
        .expect("the adapter path requires a logging object");
    assert!(matches!(error, GenAiSetupError::LoggingRequired));
    assert!(
        vertex
            .mapped
            .lock()
            .expect("the fixture lock is not poisoned")
            .is_empty()
    );
}

#[test]
fn logging_update_records_model_optional_params_and_call_metadata() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let contents = text("hello");
    let config = object(&[("responseMimeType", text("application/json"))]);
    let kwargs = object(&[("litellm_call_id", text("call-9"))]);
    let request = setup_request(
        "gemini/gemini-2.5-flash",
        &contents,
        Some(&config),
        None,
        &kwargs,
    );
    let mut state = logging();
    let result = setup_generate_content_call(&request, &configs, Some(&mut state), &resolution)
        .expect("the gemini setup succeeds");
    assert_eq!(result.logging.model, "gemini-2.5-flash");
    assert_eq!(
        result.logging.model_call_details.get("optional_params"),
        Some(&DecisionJson::Object(decision_object(&[(
            "responseMimeType",
            DecisionJson::Text("application/json".to_owned()),
        )])))
    );
    assert_eq!(
        result.logging.model_call_details.get("litellm_params"),
        Some(&DecisionJson::Object(decision_object(&[(
            "litellm_call_id",
            DecisionJson::Text("call-9".to_owned()),
        )])))
    );
    assert_eq!(
        result.logging.model_call_details.get("custom_llm_provider"),
        Some(&DecisionJson::Text("gemini".to_owned()))
    );
}

#[test]
fn logging_update_records_the_resolved_provider() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let contents = text("hello");
    let kwargs = Object::new();
    let request = setup_request("gemini/gemini-2.5-flash", &contents, None, None, &kwargs);
    let mut state = logging();
    let result = setup_generate_content_call(&request, &configs, Some(&mut state), &resolution)
        .expect("the gemini setup succeeds");
    assert_eq!(
        result.logging.custom_llm_provider.as_deref(),
        Some("gemini")
    );
}

#[test]
fn logging_update_carries_kwargs_metadata() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let contents = text("hello");
    let metadata = object(&[("user_id", text("u-1"))]);
    let kwargs = object(&[("metadata", Json::Object(metadata))]);
    let request = setup_request("gemini/gemini-2.5-flash", &contents, None, None, &kwargs);
    let mut state = logging();
    let result = setup_generate_content_call(&request, &configs, Some(&mut state), &resolution)
        .expect("the gemini setup succeeds");
    assert_eq!(
        result.logging.litellm_params.get("metadata"),
        Some(&DecisionJson::Object(decision_object(&[(
            "user_id",
            DecisionJson::Text("u-1".to_owned()),
        )])))
    );
}

#[test]
fn logging_skips_the_adapter_path() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let contents = text("hello");
    let kwargs = Object::new();
    let request = setup_request("vertex_ai/claude-sonnet-4", &contents, None, None, &kwargs);
    let mut state = logging();
    let result = setup_generate_content_call(&request, &configs, Some(&mut state), &resolution)
        .expect("the adapter fallback succeeds");
    assert_eq!(result.logging.model, "fixture-model");
    assert!(result.logging.optional_params.is_empty());
    assert_eq!(result.logging.custom_llm_provider, None);
}

#[test]
fn async_logging_marks_the_entrypoint_in_both_maps() {
    let mut state = logging();
    mark_async_entrypoint(Some(&mut state), "agenerate_content", true);
    let Some(DecisionJson::Object(params)) = state.model_call_details.get("litellm_params") else {
        panic!("the fixture starts with a litellm_params object");
    };
    assert_eq!(
        params.get("agenerate_content"),
        Some(&DecisionJson::Bool(true))
    );
    assert_eq!(
        state.litellm_params.get("agenerate_content"),
        Some(&DecisionJson::Bool(true))
    );
}

#[test]
fn async_logging_without_a_logging_object_does_nothing() {
    let mut logging: Option<LoggingState> = None;
    mark_async_entrypoint(logging.as_mut(), "agenerate_content", true);
    assert!(logging.is_none());
}

#[test]
fn request_body_is_the_transformed_request() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &gemini);
    let contents = text("hello");
    let config = object(&[("temperature", text("caller-value"))]);
    let instruction = Json::Object(object(&[("parts", Json::List(vec![text("be brief")]))]));
    let tools = Json::List(vec![text("tool")]);
    let kwargs = object(&[("systemInstruction", instruction.clone())]);
    let request = setup_request(
        "gemini/gemini-2.5-flash",
        &contents,
        Some(&config),
        Some(&tools),
        &kwargs,
    );
    let mut state = logging();
    let result = setup_generate_content_call(&request, &configs, Some(&mut state), &resolution)
        .expect("the gemini setup succeeds");
    let Json::Object(body) = result.request_body else {
        panic!("the fixture builds an object");
    };
    let generated = Json::Object(config.clone());
    assert_eq!(body.get("model"), Some(&text("gemini-2.5-flash")));
    assert_eq!(body.get("generationConfig"), Some(&generated));
    assert_eq!(body.get("tools"), Some(&tools));
    let expected = TransformedRequest {
        model: "gemini-2.5-flash".to_owned(),
        tools: Some(tools),
        generation_config: config,
        system_instruction: Some(instruction),
    };
    assert_eq!(
        *gemini
            .transformed
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
        [expected]
    );
}

#[test]
fn request_body_reads_the_snake_case_system_instruction() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let contents = text("hello");
    let instruction = Json::Object(object(&[("parts", Json::List(vec![text("be brief")]))]));
    let kwargs = object(&[("system_instruction", instruction.clone())]);
    let request = setup_request("gemini/gemini-2.5-flash", &contents, None, None, &kwargs);
    let mut state = logging();
    let result = setup_generate_content_call(&request, &configs, Some(&mut state), &resolution)
        .expect("the gemini setup succeeds");
    let body = result
        .request_body
        .as_object()
        .expect("the fixture builds an object");
    assert_eq!(body.get("systemInstruction"), Some(&instruction));
    let transformed = gemini
        .transformed
        .lock()
        .expect("the fixture lock is not poisoned");
    assert_eq!(transformed[0].system_instruction, Some(instruction));
}

#[test]
fn request_body_prefers_the_camel_case_system_instruction() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let contents = text("hello");
    let camel = Json::Object(object(&[("parts", Json::List(vec![text("camel")]))]));
    let snake = Json::Object(object(&[("parts", Json::List(vec![text("snake")]))]));
    let kwargs = object(&[
        ("systemInstruction", camel.clone()),
        ("system_instruction", snake),
    ]);
    let request = setup_request("gemini/gemini-2.5-flash", &contents, None, None, &kwargs);
    let mut state = logging();
    let result = setup_generate_content_call(&request, &configs, Some(&mut state), &resolution)
        .expect("the gemini setup succeeds");
    let body = result
        .request_body
        .as_object()
        .expect("the fixture builds an object");
    assert_eq!(body.get("systemInstruction"), Some(&camel));
    let transformed = gemini
        .transformed
        .lock()
        .expect("the fixture lock is not poisoned");
    assert_eq!(transformed[0].system_instruction, Some(camel));
}

#[test]
fn request_body_is_built_while_unread_kwargs_stay_unvalidated() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let contents = text("hello");
    let kwargs = object(&[("tpm", text("many")), ("drop_params", text("maybe"))]);
    let request = setup_request("gemini/gemini-2.5-flash", &contents, None, None, &kwargs);
    let mut state = logging();
    let result = setup_generate_content_call(&request, &configs, Some(&mut state), &resolution)
        .expect("only the fields the setup reads are validated");
    let body = result
        .request_body
        .as_object()
        .expect("the fixture builds an object");
    assert_eq!(body.get("fixture"), Some(&Json::Bool(true)));
}

#[test]
fn request_body_is_empty_for_the_adapter_path() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let gemini = FixtureConfig::for_fields(&FIELDS);
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&gemini, &vertex);
    let contents = text("hello");
    let kwargs = Object::new();
    let request = setup_request("vertex_ai/claude-sonnet-4", &contents, None, None, &kwargs);
    let mut state = logging();
    let result = setup_generate_content_call(&request, &configs, Some(&mut state), &resolution)
        .expect("the adapter fallback succeeds");
    assert_eq!(result.request_body, Json::Object(Object::new()));
    assert!(
        vertex
            .transformed
            .lock()
            .expect("the fixture lock is not poisoned")
            .is_empty()
    );
}

#[test]
fn request_body_surfaces_a_transform_failure() {
    let fixture = Fixture::new();
    let resolution = fixture.resolution();
    let failing = FixtureConfig::failing();
    let vertex = FixtureConfig::for_fields(&FIELDS);
    let configs = provider_configs(&failing, &vertex);
    let contents = text("hello");
    let kwargs = Object::new();
    let request = setup_request("gemini/gemini-2.5-flash", &contents, None, None, &kwargs);
    let mut state = logging();
    let error = setup_generate_content_call(&request, &configs, Some(&mut state), &resolution)
        .err()
        .expect("the transform failure surfaces");
    assert!(matches!(
        error,
        GenAiSetupError::Transform(FixtureError { model }) if model == "gemini-2.5-flash"
    ));
}
