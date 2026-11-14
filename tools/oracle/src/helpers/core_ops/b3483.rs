use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::vertex_schema_shaping::Settings;
use betterlite_core::genai_setup::config::{
    GenerateContentProviderConfig, GenerateContentProviderConfigs, GenerateContentRequest,
};
use betterlite_core::genai_setup::merge::merge_native_request_fields;
use betterlite_core::genai_setup::params::GenerateContentLiteLLMParams;
use betterlite_core::genai_setup::setup::{
    GenerateContentSetupResult, SetupRequest, setup_generate_content_call,
};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json as DecisionJson;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_vertex::genai::genai_contract::config::TOP_LEVEL_FIELDS;
use betterlite_vertex::google_native_content::capabilities::NativeProvider;
use betterlite_vertex::google_native_content::error::NativeContentError;
use betterlite_vertex::google_native_content::map::{
    studio_optional_params, vertex_optional_params,
};
use betterlite_vertex::google_native_content::request::{
    GenerateContentRequest as VertexRequest, transform_request as vertex_transform,
};
use betterlite_vertex::model_policy::partners::is_vertex_partner_model;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    setups: Vec<SetupCall>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SetupCall {
    model: String,
    contents: Json,
    #[serde(default)]
    config: Option<Object>,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    tools: Option<Json>,
    #[serde(default)]
    kwargs: Object,
    #[serde(default)]
    extra_body: Option<Object>,
    with_logging: bool,
}

#[derive(Debug, Error)]
enum ConfigError {
    #[error(transparent)]
    Native(#[from] NativeContentError),
    #[error("the request body cannot be written as JSON: {0}")]
    Encode(#[from] serde_json::Error),
}

#[derive(Clone, Copy)]
struct NativeConfig(NativeProvider);

impl GenerateContentProviderConfig for NativeConfig {
    type Error = ConfigError;

    fn map_optional_params(&self, config: &Object, _model: &str) -> Object {
        match self.0 {
            NativeProvider::Studio => studio_optional_params(config),
            NativeProvider::Vertex => vertex_optional_params(config),
        }
    }

    fn transform_request(&self, request: &GenerateContentRequest<'_>) -> Result<Json, Self::Error> {
        let native = VertexRequest {
            model: request.model.to_owned(),
            contents: request.contents.clone(),
            tools: request.tools.cloned().unwrap_or(Json::Null),
            generation_config: request.generation_config.clone(),
            system_instruction: request.system_instruction.cloned().unwrap_or(Json::Null),
        };
        let body = vertex_transform(self.0, &native, &Settings::default())?;
        Ok(serde_json::from_value(serde_json::to_value(body)?)?)
    }

    fn request_top_level_fields(&self) -> &'static [&'static str] {
        &TOP_LEVEL_FIELDS
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

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let fixture = Fixture::new();
        let mut answers = Vec::new();
        for call in &input.setups {
            answers.push(setup_answer(call, &fixture)?);
        }
        Ok(json!({ "setups": answers }))
    }
}

fn setup_answer(call: &SetupCall, fixture: &Fixture) -> HelperResult {
    let resolution = fixture.resolution();
    let studio = NativeConfig(NativeProvider::Studio);
    let vertex = NativeConfig(NativeProvider::Vertex);
    let configs = GenerateContentProviderConfigs {
        gemini: &studio,
        vertex: &vertex,
        is_vertex_partner_model,
    };
    let mut logging = call.with_logging.then(logging_state).transpose()?;
    let request = SetupRequest {
        model: &call.model,
        contents: &call.contents,
        config: call.config.as_ref(),
        custom_llm_provider: call.custom_llm_provider.as_deref(),
        tools: call.tools.as_ref(),
        kwargs: &call.kwargs,
    };
    let result = setup_generate_content_call(&request, &configs, logging.as_mut(), &resolution)
        .or_value_error()?;
    answer(call, result)
}

fn answer(
    call: &SetupCall,
    result: GenerateContentSetupResult<'_, '_, NativeConfig>,
) -> HelperResult {
    let native = serde_json::to_value(&result.native_request_fields)?;
    let merged =
        merge_native_request_fields(&result.native_request_fields, call.extra_body.as_ref())
            .map_or(Value::Null, |fields| json!(fields));
    let provider_config = result
        .provider_config
        .map(|config| json!(config.0.custom_llm_provider()));
    Ok(json!({
        "model": result.model,
        "provider": result.custom_llm_provider,
        "provider_config": provider_config,
        "config_dict": serde_json::to_value(&result.config_dict)?,
        "native_request_fields": native,
        "request_body": serde_json::to_value(&result.request_body)?,
        "litellm_params": params_view(&result.litellm_params),
        "merged_extra_body": merged,
        "logging": logging_view(result.logging)?,
    }))
}

fn params_view(params: &GenerateContentLiteLLMParams) -> Value {
    json!({
        "api_base": params.api_base,
        "api_version": params.api_version,
        "custom_llm_provider": params.custom_llm_provider,
        "max_retries": params.max_retries,
        "organization": params.organization,
        "api_key_set": params.api_key.is_some(),
    })
}

fn logging_view(state: &LoggingState) -> Result<Value, serde_json::Error> {
    let mut view = Map::new();
    for name in [
        "model",
        "optional_params",
        "litellm_params",
        "custom_llm_provider",
        "litellm_call_id",
    ] {
        let value = state
            .model_call_details
            .get(name)
            .cloned()
            .unwrap_or(DecisionJson::Null);
        view.insert(name.to_owned(), serde_json::to_value(value)?);
    }
    Ok(Value::Object(view))
}

fn logging_state() -> Result<LoggingState, Box<HelperError>> {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "fixture-model".to_owned(),
            call_type: "generate_content".to_owned(),
            start_time: moment()?,
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
        &redactor()?,
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .or_value_error()
}

fn moment() -> Result<DateTime<FixedOffset>, Box<HelperError>> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").or_value_error()
}

fn redactor() -> Result<Redactor, Box<HelperError>> {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()
}
