use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_context::non_default_params::mapper::OptionalParamsMapper;
use betterlite_core::responses_bridge::{
    ModelInfoError, ReasoningEffort, ResponsesBridgeCapabilities,
};
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::controls::optional_params::OptionalParams;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::params::{
    ChatParameters, ParameterError, ParameterMapping, ParameterPolicy,
};
use betterlite_models::request_pricing::register::CustomPricingRequest;
use betterlite_models::resolution::{ProviderIdentity, ResolutionError, ResolvedModel};
use betterlite_openai::openai_base_config::Object;
use secrecy::SecretString;

use crate::completion_request::args::{
    CompletionArgs, CompletionFlags, CompletionSettings, JsonObject,
};
use crate::completion_request::call::{PreparedCall, PreparedCompletion};
use crate::completion_request::error::PricingFailure;
use crate::completion_request::ports::{
    CacheSeedRequest, ChatPolicy, CompletionLogging, CompletionPorts, EnvironmentUpdate,
    ModelMetadata, ParameterMappingRequest, PricingRegistration, PromptManagement,
    PromptManagementError, PromptOutcome, PromptRequest, ProviderLookup,
};

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Lookup {
    pub model: String,
    pub provider: Option<String>,
    pub api_base: Option<String>,
    pub has_key: bool,
}

#[derive(Default)]
pub(super) struct FakeMetadata {
    pub translates_developer_role: bool,
    pub mapping: Option<ParameterMapping>,
    pub resolved_model: Option<String>,
    pub resolved_provider: Option<&'static str>,
    pub resolved_base: Option<String>,
    pub dynamic_key: Option<String>,
    pub events: Arc<Mutex<Vec<String>>>,
    pub lookups: Arc<Mutex<Vec<Lookup>>>,
    pub parameters: Arc<Mutex<Vec<ChatParameters>>>,
    pub policies: Arc<Mutex<Vec<ParameterPolicy>>>,
}

fn provider_identity(name: Option<&str>) -> ProviderIdentity {
    match name {
        Some("anthropic") => ProviderIdentity::Known(ProviderId::Anthropic),
        Some("azure") => ProviderIdentity::Known(ProviderId::Azure),
        Some(other) => ProviderIdentity::Custom(other.to_owned()),
        None => ProviderIdentity::Known(ProviderId::Openai),
    }
}

impl ModelMetadata for FakeMetadata {
    fn resolve_provider(
        &self,
        request: &ProviderLookup<'_>,
    ) -> Result<ResolvedModel, ResolutionError> {
        lock(&self.events).push(format!("resolve:{}", request.model));
        lock(&self.lookups).push(Lookup {
            model: request.model.to_owned(),
            provider: request.custom_llm_provider.map(str::to_owned),
            api_base: request.api_base.map(str::to_owned),
            has_key: request.api_key.is_some(),
        });
        Ok(ResolvedModel {
            model: self
                .resolved_model
                .clone()
                .unwrap_or_else(|| request.model.to_owned()),
            provider: provider_identity(self.resolved_provider.or(request.custom_llm_provider)),
            api_base: self
                .resolved_base
                .clone()
                .or_else(|| request.api_base.map(str::to_owned)),
            api_key: self
                .dynamic_key
                .as_ref()
                .map(|key| SecretString::from(key.clone()))
                .or_else(|| request.api_key.cloned()),
        })
    }

    fn chat_policy<'config>(
        &'config self,
        _model: &str,
        _provider: &str,
        _base_model: Option<&str>,
    ) -> ChatPolicy<'config> {
        ChatPolicy {
            translates_developer_role: self.translates_developer_role,
            mapper: None::<&'config dyn OptionalParamsMapper>,
        }
    }

    fn parameter_mapping(
        &self,
        request: &ParameterMappingRequest<'_>,
    ) -> Result<ParameterMapping, ParameterError> {
        lock(&self.events).push(String::from("mapping"));
        lock(&self.parameters).push(request.parameters.clone());
        lock(&self.policies).push(request.policy.clone());
        Ok(self.mapping.clone().unwrap_or_default())
    }
}

#[derive(Default)]
pub(super) struct FakePrompt {
    pub should_run: bool,
    pub outcome: Option<PromptOutcome>,
    pub seed_value: Option<Json>,
    pub events: Arc<Mutex<Vec<String>>>,
    pub seeded: Arc<Mutex<Vec<JsonObject>>>,
    pub hook_parameters: Arc<Mutex<Vec<JsonObject>>>,
}

impl FakePrompt {
    fn recorded(&self, event: &str, request: PromptRequest) -> PromptOutcome {
        lock(&self.events).push(event.to_owned());
        self.outcome
            .clone()
            .unwrap_or_else(|| prompt_outcome(request))
    }
}

fn prompt_outcome(request: PromptRequest) -> PromptOutcome {
    PromptOutcome {
        model: request.model,
        messages: request.messages,
        non_default_params: request.non_default_params,
    }
}

#[async_trait]
impl PromptManagement for FakePrompt {
    fn seed_cache_control(
        &self,
        _request: &CacheSeedRequest<'_>,
        non_default_params: &mut JsonObject,
    ) {
        lock(&self.events).push(String::from("seed"));
        if let Some(value) = &self.seed_value {
            non_default_params.insert(
                String::from("cache_control_injection_points"),
                value.clone(),
            );
        }
        lock(&self.seeded).push(non_default_params.clone());
    }

    fn should_run(
        &self,
        _prompt_id: Option<&str>,
        non_default_params: &JsonObject,
        _tools: Option<&[Json]>,
    ) -> bool {
        lock(&self.events).push(String::from("should_run"));
        lock(&self.hook_parameters).push(non_default_params.clone());
        self.should_run
    }

    async fn get_chat_completion_prompt(
        &self,
        request: PromptRequest,
    ) -> Result<PromptOutcome, PromptManagementError> {
        Ok(self.recorded("hook_sync", request))
    }

    async fn async_completion_prompt(
        &self,
        request: PromptRequest,
    ) -> Result<PromptOutcome, PromptManagementError> {
        lock(&self.events).push(String::from("hook_async"));
        self.get_chat_completion_prompt(request).await
    }
}

#[derive(Default)]
pub(super) struct FakeBridge {
    pub mode: Option<String>,
    pub gpt5: Option<bool>,
    pub gpt5_4: Option<bool>,
    pub search: Option<bool>,
    pub events: Arc<Mutex<Vec<String>>>,
}

impl ResponsesBridgeCapabilities for FakeBridge {
    fn model_info(&self, _model: &str, _provider: &str) -> Result<Object, ModelInfoError> {
        lock(&self.events).push(String::from("bridge_info"));
        let text = self.mode.as_ref().map_or_else(
            || String::from("{}"),
            |mode| format!("{{\"mode\":\"{mode}\"}}"),
        );
        serde_json::from_str(&text).map_err(|_error| ModelInfoError::Unknown {
            model: String::from("fixture"),
        })
    }

    fn resolved_openai_api_base(&self, api_base: Option<&str>) -> String {
        api_base.unwrap_or("https://api.openai.com/v1").to_owned()
    }

    fn foundry_supports_responses(&self, _model: &str, _api_base: Option<&str>) -> bool {
        false
    }

    fn foundry_chat_rejects_tools(
        &self,
        _model: &str,
        _reasoning_effort: Option<&ReasoningEffort>,
    ) -> bool {
        false
    }

    fn is_gpt_5_model(&self, _model: &str) -> bool {
        self.gpt5.unwrap_or(false)
    }

    fn is_gpt_5_4_plus_model(&self, _model: &str) -> bool {
        self.gpt5_4.unwrap_or(false)
    }

    fn is_gpt_5_search_model(&self, _model: &str) -> bool {
        self.search.unwrap_or(false)
    }
}

#[derive(Default)]
pub(super) struct FakePricing {
    pub requests: Arc<Mutex<Vec<(String, String)>>>,
    pub events: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl PricingRegistration for FakePricing {
    async fn register_custom_pricing(
        &self,
        request: &CustomPricingRequest<'_>,
    ) -> Result<(), PricingFailure> {
        lock(&self.events).push(String::from("pricing"));
        lock(&self.requests).push((
            request.model.to_owned(),
            request.custom_llm_provider.to_owned(),
        ));
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LoggedEnvironment {
    pub model: String,
    pub user: Option<String>,
    pub provider: Option<String>,
    pub optional_params: JsonObject,
}

#[derive(Default)]
pub(super) struct FakeLogging {
    pub updates: Arc<Mutex<Vec<LoggedEnvironment>>>,
    pub events: Arc<Mutex<Vec<String>>>,
}

impl CompletionLogging for FakeLogging {
    fn update_environment(&self, update: &EnvironmentUpdate<'_>) {
        lock(&self.events).push(String::from("log"));
        lock(&self.updates).push(LoggedEnvironment {
            model: update.model.to_owned(),
            user: update.user.map(str::to_owned),
            provider: update.custom_llm_provider.map(str::to_owned),
            optional_params: update.optional_params.clone(),
        });
    }
}

pub(super) struct Fixture {
    pub args: CompletionArgs,
    pub settings: CompletionSettings,
    pub metadata: FakeMetadata,
    pub prompt: FakePrompt,
    pub bridge: FakeBridge,
    pub pricing: FakePricing,
    pub logging: FakeLogging,
    pub events: Arc<Mutex<Vec<String>>>,
}

pub(super) fn args_from(text: &str) -> CompletionArgs {
    serde_json::from_str(text).unwrap()
}

pub(super) fn settings_default() -> CompletionSettings {
    CompletionSettings {
        drop_params: None,
        model_fallbacks: None,
        model_alias_map: BTreeMap::new(),
        num_retries: None,
        request_timeout: None,
        api_key: None,
        api_base: None,
        route_all_to_responses: false,
        proxy_headers: AdditionalHeaders::new(),
        optional: OptionalParams::default(),
        flags: CompletionFlags {
            add_function_to_prompt: false,
            disable_stop_sequence_limit: false,
        },
        medium_thinking_budget: 2048,
    }
}

pub(super) fn message(role: &str, content: &str) -> Json {
    Json::Object(JsonObject::from([
        (String::from("role"), Json::Text(role.to_owned())),
        (String::from("content"), Json::Text(content.to_owned())),
    ]))
}

fn pricing_fake(events: Arc<Mutex<Vec<String>>>) -> FakePricing {
    FakePricing {
        requests: Arc::new(Mutex::new(Vec::new())),
        events,
    }
}

fn logging_fake(events: Arc<Mutex<Vec<String>>>) -> FakeLogging {
    FakeLogging {
        updates: Arc::new(Mutex::new(Vec::new())),
        events,
    }
}

impl Fixture {
    pub(super) fn new() -> Self {
        let events = Arc::new(Mutex::new(Vec::new()));
        Self {
            args: args_from(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#),
            settings: settings_default(),
            metadata: FakeMetadata {
                translates_developer_role: false,
                mapping: None,
                resolved_model: None,
                resolved_provider: None,
                resolved_base: None,
                dynamic_key: None,
                events: Arc::clone(&events),
                lookups: Arc::new(Mutex::new(Vec::new())),
                parameters: Arc::new(Mutex::new(Vec::new())),
                policies: Arc::new(Mutex::new(Vec::new())),
            },
            prompt: FakePrompt {
                should_run: false,
                outcome: None,
                seed_value: None,
                events: Arc::clone(&events),
                seeded: Arc::new(Mutex::new(Vec::new())),
                hook_parameters: Arc::new(Mutex::new(Vec::new())),
            },
            bridge: FakeBridge {
                mode: None,
                gpt5: None,
                gpt5_4: None,
                search: None,
                events: Arc::clone(&events),
            },
            pricing: pricing_fake(Arc::clone(&events)),
            logging: logging_fake(Arc::clone(&events)),
            events,
        }
    }

    pub(super) fn ports(&self) -> CompletionPorts<'_> {
        CompletionPorts {
            model_metadata: &self.metadata,
            prompt: &self.prompt,
            bridge: &self.bridge,
            pricing: &self.pricing,
            logging: &self.logging,
        }
    }

    pub(super) fn events(&self) -> Vec<String> {
        lock(&self.events).clone()
    }
}

pub(super) fn call_of(outcome: PreparedCompletion) -> PreparedCall {
    match outcome {
        PreparedCompletion::Dispatch(call)
        | PreparedCompletion::ResponsesBridge(call)
        | PreparedCompletion::McpLoop(call) => call,
        other @ (PreparedCompletion::Fallbacks(_)
        | PreparedCompletion::ModelList(_)
        | PreparedCompletion::Mock(_)) => panic!("expected a prepared call, got {other:?}"),
    }
}

pub(super) fn parameters(state: &FakeMetadata) -> Vec<ChatParameters> {
    lock(&state.parameters).clone()
}

pub(super) fn lookups(metadata: &FakeMetadata) -> Vec<Lookup> {
    lock(&metadata.lookups).clone()
}

pub(super) fn hook_parameters(prompt: &FakePrompt) -> Vec<JsonObject> {
    lock(&prompt.hook_parameters).clone()
}

pub(super) fn mapping_with(
    standard: &[(&str, Json)],
    provider_fields: &[(&str, Json)],
) -> ParameterMapping {
    let mut mapping = ParameterMapping::default();
    for (name, value) in standard {
        mapping
            .standard
            .insert((*name).to_owned(), serde_json::to_value(value).unwrap());
    }
    for (name, value) in provider_fields {
        mapping
            .provider_fields
            .insert((*name).to_owned(), serde_json::to_value(value).unwrap());
    }
    mapping
}
