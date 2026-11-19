use std::fmt::{self, Formatter};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::model_capability_predicates::{
    CapabilityPredicates, CapabilityRequest, NamedCapability,
};
use betterlite_openai::chat_choices::endpoint::validate_environment as gpt_environment;
use betterlite_openai::chat_choices::stream::StreamingHandler;
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, ModelFamily, Object, OpenAiConfig, OpenAiSettings,
    model_family,
};
use futures_util::FutureExt as _;
use secrecy::SecretString;

use crate::aws_settings::credential_cache::CredentialCache;
use crate::bedrock_settings::{BedrockError, HeaderError};
use crate::messages::transport::get_error_class as bedrock_error;

use super::error::MantleSettingsError;

pub const CUSTOM_LLM_PROVIDER: ProviderId = ProviderId::BedrockMantle;
pub const PROJECT_HEADER: &str = "OpenAI-Project";

// BedrockMantleChatConfig.__init__: the inherited OpenAI-like GPT initialization takes no argument here, and a truthy injected signer is kept.
pub struct MantleChatConfig {
    pub openai: OpenAiConfig,
    pub credential_cache: Option<Arc<CredentialCache>>,
}

impl fmt::Debug for MantleChatConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("MantleChatConfig")
            .field("openai", &self.openai)
            .field("has_credential_cache", &self.credential_cache.is_some())
            .finish()
    }
}

impl MantleChatConfig {
    #[must_use]
    pub fn new(credential_cache: Option<Arc<CredentialCache>>) -> Self {
        // OpenAILikeChatConfig is not the OpenAI family, so no inherited parameter carries reasoning_effort.
        let openai = OpenAiConfig {
            family: ConfigFamily::Other,
            api_key: None,
            headers: Headers::new(),
            settings: OpenAiSettings::default(),
        };
        Self {
            openai,
            credential_cache,
        }
    }

    // get_config returns the inherited class attributes; this class holds only defaults, so the answer is empty.
    // Defaults serialize to an empty object, so an empty Ok is the same answer; no test can tell them apart.
    #[cfg_attr(test, mutants::skip)]
    pub fn get_config(&self) -> Result<Object, MantleSettingsError> {
        let attributes =
            serde_json::to_value(&self.openai.settings).map_err(MantleSettingsError::Encode)?;
        serde_json::from_value(attributes).map_err(|_error| MantleSettingsError::Attributes)
    }

    // Inherited GPT validation, then a truthy aws_bedrock_project_id overwrites the exact OpenAI-Project header; no bearer fallback runs here.
    #[must_use]
    pub fn validate_environment(
        &self,
        api_key: Option<&SecretString>,
        headers: &Headers,
        project_id: Option<&str>,
    ) -> Headers {
        let mut merged = gpt_environment(api_key, headers);
        if let Some(project) = project_id.filter(|value| !value.is_empty()) {
            merged.insert(PROJECT_HEADER.to_owned(), project.to_owned());
        }
        merged
    }

    // The inherited GPT list, then verbosity for a gpt reasoning series name, then reasoning_effort when the capability policy says so; each only when absent from the base list.
    #[must_use]
    pub async fn supported_params<Policy>(
        &self,
        model: &str,
        catalog: &CatalogCapabilities,
        policy: &mut Policy,
    ) -> Vec<String>
    where
        Policy: CapabilityPredicates,
    {
        let base = self.openai.supported_params(model, catalog);
        let mut params: Vec<String> = base.iter().map(|param| (*param).to_owned()).collect();
        if model_family(model, catalog) == ModelFamily::GptReasoning && !base.contains(&"verbosity")
        {
            params.push("verbosity".to_owned());
        }
        if !base.contains(&"reasoning_effort") && supports_reasoning(model, policy).await {
            params.push("reasoning_effort".to_owned());
        }
        params
    }

    // OpenAIChatCompletionStreamingHandler keeps only json_mode; sync_stream is accepted and has no effect (B504).
    #[must_use]
    pub fn response_iterator(_sync_stream: bool, json_mode: Option<bool>) -> StreamingHandler {
        match json_mode {
            Some(true) => StreamingHandler { json_mode: true },
            Some(false) | None => StreamingHandler { json_mode: false },
        }
    }

    // BedrockError takes the supplied message, status and headers and adds no type, param or code (B622).
    pub fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> Result<BedrockError, HeaderError> {
        let pairs: Vec<(String, String)> = headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        bedrock_error(message, status_code, &pairs)
    }
}

// BedrockMantleChatConfig._supports_reasoning: the B688 policy answers for provider bedrock_mantle, and any failure is logged at debug and read as false.
pub async fn supports_reasoning<Policy>(model: &str, policy: &mut Policy) -> bool
where
    Policy: CapabilityPredicates,
{
    let provider = <&'static str>::from(CUSTOM_LLM_PROVIDER);
    let request = CapabilityRequest {
        model,
        custom_llm_provider: Some(provider),
    };
    let probe = AssertUnwindSafe(async move {
        policy
            .supports_factory(&request, NamedCapability::Reasoning.key())
            .await
    });
    match probe.catch_unwind().await {
        Ok(supported) => supported,
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .map(|text| (*text).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "non-string panic payload".to_owned());
            tracing::debug!("BedrockMantleChatConfig: error checking reasoning support: {message}");
            false
        }
    }
}
