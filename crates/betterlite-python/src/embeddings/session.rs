use super::convert::{ConvertError, EmbeddingArguments, EmbeddingSettingsWire, SessionWire};
use super::error::{CallFacts, CallFailure, Raised, SessionError, raised, resolution_failure};
use crate::completion::setup::{ModuleCredentials, openai_defaults, outbound_client};
use betterlite_config::loader::startup::process_environment;
use betterlite_config::types::{LiteLlmParams, ModelDeployment, ModelInfo};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, ResolvedModel, resolve};
use betterlite_openai::chat::deployment::{ChatDeployment, OpenAiDefaults};
use betterlite_openai::openai_base_config::{CatalogCapabilities, Headers};
use betterlite_openai::text_completion_execution::catalog::CompletionCatalog;
use betterlite_provider::prepared::PreparedRequest;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
// Everything one Python session owns: the environment snapshot, the HTTP client and the catalog it resolves against.
pub(crate) struct EmbeddingSession {
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) catalog: Arc<ModelCatalog>,
    pub(crate) known_model_names: BTreeSet<String>,
    pub(crate) custom_providers: BTreeSet<String>,
    pub(crate) registry: JSONProviderRegistry,
    pub(crate) defaults: OpenAiDefaults,
    pub(crate) client: ProxiedClient,
    pub(crate) capabilities: CatalogCapabilities,
    pub(crate) redactor: Redactor,
}
impl EmbeddingSession {
    pub(crate) fn build(text: &str) -> Result<Self, SessionError> {
        let wire: SessionWire =
            serde_json::from_str(text).map_err(|source| ConvertError::Json { source })?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let client = outbound_client(&environment)?;
        let catalog = BundledCatalog::default().get();
        let completion = CompletionCatalog::new(&catalog);
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
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
        })
    }
    // get_llm_provider, run over the snapshot this session holds; an empty base counts as unset
    pub(crate) fn resolve(
        &self,
        arguments: &EmbeddingArguments,
        api_key: Option<&SecretString>,
        globals: &GlobalCredentials,
    ) -> Result<ResolvedModel, CallFailure> {
        let custom_llm_provider = if arguments.azure == Some(true) {
            Some("azure")
        } else {
            arguments.custom_llm_provider.as_deref()
        };
        let request = ResolutionRequest {
            model: &arguments.model,
            custom_llm_provider,
            api_base: arguments
                .api_base
                .as_deref()
                .filter(|text| !text.is_empty()),
            api_key,
        };
        resolve(&request, &self.context(globals))
            .map_err(|error| resolution_failure(&error, &arguments.model))
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
}
impl ModuleCredentials for EmbeddingSettingsWire {
    fn api_key(&self) -> Option<SecretString> {
        secret_text(self.api_key.as_ref())
    }
    fn api_base(&self) -> Option<String> {
        self.api_base.clone().filter(|text| !text.is_empty())
    }
}
// get_llm_provider's answer next to the call's own key and arguments
pub(crate) struct DeploymentInputs<'call> {
    pub(crate) resolved: &'call ResolvedModel,
    pub(crate) facts: &'call CallFacts<'call>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) arguments: &'call EmbeddingArguments,
    pub(crate) globals: &'call GlobalCredentials,
}
// get_llm_provider's inputs, turned into the deployment the shared provider pipeline prepares
pub(crate) fn deployment(
    session: &EmbeddingSession,
    inputs: &DeploymentInputs<'_>,
) -> Result<ChatDeployment, CallFailure> {
    ChatDeployment::resolve_with(
        &declared(inputs),
        &session.defaults,
        None,
        &session.context(inputs.globals),
    )
    .map_err(|error| {
        raised(
            Raised::BadRequest,
            error.to_string(),
            Some(inputs.facts.model.to_owned()),
            Some(inputs.facts.provider.to_owned()),
        )
    })
}
// the deployment a router entry would declare for this call's resolved model, key and version
fn declared(inputs: &DeploymentInputs<'_>) -> ModelDeployment {
    ModelDeployment {
        model_name: inputs.resolved.model.clone(),
        litellm_params: LiteLlmParams {
            ssl_verify: None,
            model: Some(format!("{}/{}", inputs.facts.provider, inputs.facts.model)),
            api_base: inputs
                .resolved
                .api_base
                .clone()
                .filter(|text| !text.is_empty()),
            api_key: inputs
                .resolved
                .api_key
                .clone()
                .or_else(|| inputs.api_key.clone()),
            api_version: inputs.arguments.api_version.clone(),
            timeout: inputs.arguments.timeout,
            stream_timeout: None,
            max_retries: None,
            rpm: None,
            tpm: None,
            weight: None,
        },
        // the dispatch looks a deployment up by its model id, which LiteLLM's model_info.id pins
        model_info: ModelInfo {
            id: Some(inputs.resolved.model.clone()),
        },
    }
}
// LiteLLM's client puts the caller's headers on the request after its own, so they win a shared name
pub(crate) fn with_headers(
    prepared: PreparedRequest,
    headers: Option<&Headers>,
) -> Result<PreparedRequest, CallFailure> {
    let Some(caller_headers) = headers.filter(|given| !given.is_empty()) else {
        return Ok(prepared);
    };
    let PreparedRequest::Embedding(mut chat) = prepared else {
        return Err(CallFailure::Invalid(String::from(
            "the resolved provider's request cannot carry the caller's headers yet",
        )));
    };
    for (name, value) in caller_headers {
        let header = HeaderName::try_from(name.as_str())
            .map_err(|error| CallFailure::Invalid(format!("{name}: {error}")))?;
        let text = HeaderValue::try_from(value.as_str())
            .map_err(|error| CallFailure::Invalid(format!("{name}: {error}")))?;
        chat.http.headers.insert(header, text);
    }
    Ok(PreparedRequest::Embedding(chat))
}
pub(crate) fn header_map(headers: &HeaderMap) -> BTreeMap<String, String> {
    let mut collected = BTreeMap::new();
    for (name, value) in headers {
        if let Ok(text) = value.to_str() {
            collected.insert(name.as_str().to_owned(), text.to_owned());
        }
    }
    collected
}
// LiteLLM's `value or next`: an empty key counts as unset
pub(crate) fn secret_text(value: Option<&SecretString>) -> Option<SecretString> {
    value
        .filter(|secret| !secret.expose_secret().is_empty())
        .cloned()
}
