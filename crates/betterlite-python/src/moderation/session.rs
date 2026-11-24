use super::convert::{ConvertError, OPENAI_PROVIDER, SessionWire, SettingsWire, TextArguments};
use super::error::{CallFailure, Raised, SessionError, raised, resolution_failure};
use crate::completion::setup::{ModuleCredentials, openai_defaults, outbound_client};
use betterlite_config::loader::startup::process_environment;
use betterlite_config::types::{LiteLlmParams, ModelDeployment, ModelInfo};
use betterlite_core::mock::chat::response as mock_chat;
use betterlite_core::mock::embedding::response as mock_embedding;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, ResolvedModel, resolve};
use betterlite_openai::chat::MockSettings;
use betterlite_openai::chat::deployment::{ChatDeployment, OpenAiDefaults};
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_openai::text_completion_execution::CompletionExecutor;
use betterlite_openai::text_completion_execution::catalog::CompletionCatalog;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use secrecy::{ExposeSecret as _, SecretString};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use tokio::sync::OnceCell;
use uuid::Uuid;

// Which entry point asked, because main.py's sync and async moderation read the model differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entry {
    Sync,
    Async,
}

pub(crate) struct TextSession {
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) catalog: Arc<ModelCatalog>,
    pub(crate) known_model_names: BTreeSet<String>,
    pub(crate) custom_providers: BTreeSet<String>,
    pub(crate) registry: JSONProviderRegistry,
    pub(crate) defaults: OpenAiDefaults,
    pub(crate) client: ProxiedClient,
    pub(crate) capabilities: CatalogCapabilities,
    pub(crate) redactor: Redactor,
    pub(crate) executor: CompletionExecutor,
}

impl TextSession {
    pub(crate) fn build(text: &str) -> Result<Self, SessionError> {
        let wire: SessionWire =
            serde_json::from_str(text).map_err(|source| ConvertError::Json { source })?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let client = outbound_client(&environment)?;
        let catalog = BundledCatalog::default().get();
        let completion = CompletionCatalog::new(&catalog);
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
        let executor = CompletionExecutor {
            client: client.clone(),
            bundled_catalog: Arc::clone(&catalog),
            catalog: OnceCell::new(),
        };
        Ok(Self {
            defaults: openai_defaults(&environment),
            environment,
            catalog,
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            registry: JSONProviderRegistry::load(),
            client,
            capabilities: CatalogCapabilities {
                chat_models: completion.chat_models,
                text_models: completion.text_models,
            },
            redactor,
            executor,
        })
    }

    // get_llm_provider, run over the snapshot this session holds; an empty base counts as unset
    pub(crate) fn resolve(
        &self,
        requested: &str,
        arguments: &TextArguments,
        api_key: Option<&SecretString>,
        globals: &GlobalCredentials,
    ) -> Result<ResolvedModel, CallFailure> {
        let request = ResolutionRequest {
            model: requested,
            custom_llm_provider: arguments.custom_llm_provider.as_deref(),
            api_base: arguments
                .api_base
                .as_deref()
                .filter(|text| !text.is_empty()),
            api_key,
        };
        resolve(&request, &self.context(globals))
            .map_err(|error| resolution_failure(&error, requested))
    }

    pub(crate) fn context(&self, globals: &GlobalCredentials) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known_model_names,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: self.registry.clone(),
            policies: ProviderPolicies::default(),
            globals: globals.clone(),
        }
    }

    // moderation always calls the openai moderations endpoint; the model keeps no provider prefix of its own
    pub(crate) fn moderation_deployment(
        &self,
        model: &str,
        api_key: Option<SecretString>,
        api_base: Option<String>,
    ) -> Result<ChatDeployment, CallFailure> {
        let deployment = ModelDeployment {
            model_name: model.to_owned(),
            litellm_params: LiteLlmParams {
                ssl_verify: None,
                model: Some(format!("openai/{model}")),
                api_base: api_base.filter(|text| !text.is_empty()),
                api_key,
                api_version: None,
                timeout: None,
                stream_timeout: None,
                max_retries: None,
                rpm: None,
                tpm: None,
                weight: None,
            },
            model_info: ModelInfo {
                id: Some(model.to_owned()),
            },
        };
        let globals = GlobalCredentials::default();
        self.resolved(&deployment, model, OPENAI_PROVIDER, &globals)
    }

    pub(crate) fn resolved(
        &self,
        deployment: &ModelDeployment,
        model: &str,
        provider: &str,
        globals: &GlobalCredentials,
    ) -> Result<ChatDeployment, CallFailure> {
        ChatDeployment::resolve_with(deployment, &self.defaults, None, &self.context(globals))
            .map_err(|error| {
                raised(
                    Raised::BadRequest,
                    error.to_string(),
                    Some(model.to_owned()),
                    Some(provider.to_owned()),
                )
            })
    }

    pub(crate) fn chat_executor(
        &self,
        deployment: &ChatDeployment,
    ) -> Result<ChatExecutor, CallFailure> {
        Ok(ChatExecutor {
            client: self.client.clone(),
            deployments: vec![deployment.clone()],
            catalog: self.capabilities.clone(),
            random: Mutex::new(StdRng::seed_from_u64(0_u64)),
            clock: || 0_u64,
            next_id: Uuid::new_v4,
            mock: MockSettings::default(),
            mock_chat,
            mock_embedding,
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?,
            bundled_catalog: Arc::clone(&self.catalog),
        })
    }
}

pub(crate) struct DeploymentFacts<'call> {
    pub(crate) resolved: &'call ResolvedModel,
    pub(crate) arguments: &'call TextArguments,
    pub(crate) model: &'call str,
    pub(crate) provider: &'call str,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) globals: &'call GlobalCredentials,
}

pub(crate) fn declared(facts: &DeploymentFacts<'_>) -> ModelDeployment {
    let api_base = facts.resolved.api_base.clone();
    let api_key = facts.resolved.api_key.clone();
    ModelDeployment {
        model_name: facts.model.to_owned(),
        litellm_params: LiteLlmParams {
            ssl_verify: None,
            model: Some(format!("{}/{}", facts.provider, facts.model)),
            api_base: api_base.filter(|text| !text.is_empty()),
            api_key: api_key.or_else(|| facts.api_key.clone()),
            api_version: facts.arguments.api_version.clone(),
            timeout: facts.arguments.timeout,
            stream_timeout: None,
            max_retries: None,
            rpm: None,
            tpm: None,
            weight: None,
        },
        // the dispatch looks a deployment up by its model id, which LiteLLM's model_info.id pins
        model_info: ModelInfo {
            id: Some(facts.model.to_owned()),
        },
    }
}

impl ModuleCredentials for SettingsWire {
    fn api_key(&self) -> Option<SecretString> {
        secret_text(self.api_key.as_ref())
    }
    fn api_base(&self) -> Option<String> {
        self.api_base.clone().filter(|text| !text.is_empty())
    }
}

// LiteLLM's `value or next`: an empty key counts as unset
pub(crate) fn secret_text(value: Option<&SecretString>) -> Option<SecretString> {
    value
        .filter(|secret| !secret.expose_secret().is_empty())
        .cloned()
}
