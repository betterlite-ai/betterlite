use super::convert::{EmbeddingArguments, EmbeddingSettingsWire, SessionWire, parse};
use super::error::{CallFacts, CallFailure, Raised, SessionError, raised};
use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::{LiteLlmParams, ModelDeployment, ModelInfo, ProxyTrust};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::call_helpers::headers::{
    AdditionalHeaders, HeaderSource, process_response_headers,
};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::{
    ResolutionContext, ResolutionError, ResolutionRequest, ResolvedModel, resolve,
};
use betterlite_openai::chat::deployment::{ChatDeployment, OpenAiDefaults};
use betterlite_openai::openai_base_config::{CatalogCapabilities, Headers};
use betterlite_openai::text_completion_execution::catalog::CompletionCatalog;
use betterlite_provider::prepared::PreparedRequest;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
// get_litellm_params' default before main.py's OpenAI branch replaces it with the /v1 base
const OPENAI_BASE: &str = "https://api.openai.com";
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
        let wire: SessionWire = parse(text)?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let client = outbound_client(&environment)?;
        let catalog = BundledCatalog::default().get();
        let completion = CompletionCatalog::new(&catalog);
        let redactor =
            Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| SessionError::Redact {
                message: error.to_string(),
            })?;
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
    // get_llm_provider, run over the snapshot this session holds
    pub(crate) fn resolve(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
        globals: &GlobalCredentials,
    ) -> Result<ResolvedModel, ResolutionError> {
        resolve(
            &ResolutionRequest {
                model,
                custom_llm_provider,
                api_base,
                api_key,
            },
            &self.context(globals),
        )
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
fn first_text(environment: &BTreeMap<String, String>, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        environment
            .get(*name)
            .filter(|value| !value.is_empty())
            .cloned()
    })
}
// the environment values main.py's OpenAI branch falls back to, in its order
pub(crate) fn openai_defaults(environment: &BTreeMap<String, String>) -> OpenAiDefaults {
    OpenAiDefaults {
        request_timeout: None,
        api_bases: first_text(environment, &["OPENAI_BASE_URL", "OPENAI_API_BASE"])
            .into_iter()
            .collect(),
        api_keys: first_text(environment, &["OPENAI_API_KEY"])
            .map(SecretString::from)
            .into_iter()
            .collect(),
        organization: first_text(environment, &["OPENAI_ORGANIZATION"]),
        global_drop_params: false,
    }
}
// the outbound client honours the proxy variables of the environment the session was built from
fn outbound_client(environment: &BTreeMap<String, String>) -> Result<ProxiedClient, SessionError> {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), environment)
        .map_err(|source| SessionError::Proxy { source })?;
    ProxiedClient::outbound(&Arc::new(policy)).map_err(|source| SessionError::Client { source })
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
    inputs: DeploymentInputs<'_>,
) -> Result<ChatDeployment, CallFailure> {
    let DeploymentInputs {
        resolved,
        facts,
        api_key,
        arguments,
        globals,
    } = inputs;
    let declared = ModelDeployment {
        model_name: resolved.model.clone(),
        litellm_params: LiteLlmParams {
            ssl_verify: None,
            model: Some(format!("{}/{}", facts.provider, facts.model)),
            api_base: plain_text(resolved.api_base.as_deref()),
            api_key: resolved.api_key.clone().or(api_key),
            api_version: plain_text(arguments.api_version.as_deref()),
            timeout: arguments.timeout,
            stream_timeout: None,
            max_retries: None,
            rpm: None,
            tpm: None,
            weight: None,
        },
        // the dispatch looks a deployment up by its model id, which LiteLLM's model_info.id pins
        model_info: ModelInfo {
            id: Some(resolved.model.clone()),
        },
    };
    ChatDeployment::resolve_with(
        &declared,
        &session.defaults,
        None,
        &session.context(globals),
    )
    .map_err(|error| {
        raised(
            Raised::BadRequest,
            error.to_string(),
            Some(facts.model.to_owned()),
            Some(facts.provider.to_owned()),
        )
    })
}
// LiteLLM's client puts the caller's headers on the request after its own, so they win a shared name
pub(crate) fn with_headers(
    prepared: PreparedRequest,
    headers: Option<&Headers>,
) -> Result<PreparedRequest, CallFailure> {
    let Some(headers) = headers else {
        return Ok(prepared);
    };
    if headers.is_empty() {
        return Ok(prepared);
    }
    let PreparedRequest::Embedding(mut chat) = prepared else {
        return Err(CallFailure::Invalid(String::from(
            "the resolved provider's request cannot carry the caller's headers yet",
        )));
    };
    for (name, value) in headers {
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
// a_litellm_logging's additional_headers: the six rate-limit names bare, every other name prefixed
pub(crate) fn additional_headers(headers: &BTreeMap<String, String>) -> AdditionalHeaders {
    let mut source = IndexMap::new();
    for (name, value) in headers {
        source.insert(name.clone(), value.clone());
    }
    process_response_headers(HeaderSource::Upstream(&source))
}
// the base main.py's OpenAI branch falls back to before the handler defaults to https://api.openai.com/v1
pub(crate) fn mock_api_base(
    arguments: &EmbeddingArguments,
    settings: &EmbeddingSettingsWire,
    environment: &BTreeMap<String, String>,
) -> String {
    plain_text(arguments.api_base.as_deref())
        .or_else(|| plain_text(settings.api_base.as_deref()))
        .or_else(|| environment_text(environment, "OPENAI_BASE_URL"))
        .or_else(|| environment_text(environment, "OPENAI_API_BASE"))
        .unwrap_or_else(|| String::from(OPENAI_BASE))
}
// LiteLLM's `value or next`: an empty key or base counts as unset
pub(crate) fn plain_text(value: Option<&str>) -> Option<String> {
    value.filter(|text| !text.is_empty()).map(str::to_owned)
}
pub(crate) fn secret_text(value: Option<&SecretString>) -> Option<SecretString> {
    value
        .filter(|secret| !secret.expose_secret().is_empty())
        .cloned()
}
fn environment_text(environment: &BTreeMap<String, String>, name: &str) -> Option<String> {
    environment
        .get(name)
        .filter(|value| !value.is_empty())
        .cloned()
}
