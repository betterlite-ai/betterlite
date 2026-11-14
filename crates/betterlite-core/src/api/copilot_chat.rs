use betterlite_config::sections::copilot_chat::Settings;
use betterlite_models::model_capability_predicates::{
    CapabilityPredicates, CapabilityRequest, NamedCapability,
};
use betterlite_openai::copilot::api_key::error::ApiKeyError;
use betterlite_openai::copilot::headers::{DEFAULT_COPILOT_API_BASE, copilot_default_headers};
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};
use secrecy::SecretString;
use uuid::Uuid;

pub mod credentials;
pub mod error;
pub mod messages;
pub mod native_reply;
pub mod reply;

use credentials::CopilotCredentials;
use error::CopilotChatError;
use messages::{CopilotMessage, has_vision_content, initiator};

mod tests;

#[derive(Debug, Clone)]
pub struct ProviderInfo {
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub provider: String,
}

// LiteLLM's GithubCopilotConfig: the OpenAI policy underneath, Copilot credentials and settings on top
pub struct GithubCopilotConfig<A, P> {
    pub base: OpenAiConfig,
    pub credentials: A,
    pub predicates: P,
    pub settings: Settings,
    pub catalog: CatalogCapabilities,
}

impl<A, P> GithubCopilotConfig<A, P>
where
    A: CopilotCredentials + Send,
    P: CapabilityPredicates + Send,
{
    // the subclass skips the unknown-model reasoning_effort branch of OpenAIConfig, which ConfigFamily::Other stands for
    #[must_use]
    pub fn new(
        api_key: Option<SecretString>,
        headers: Headers,
        credentials: A,
        predicates: P,
        settings: Settings,
        catalog: CatalogCapabilities,
    ) -> Self {
        Self {
            base: OpenAiConfig {
                family: ConfigFamily::Other,
                api_key,
                headers,
                settings: OpenAiSettings::default(),
            },
            credentials,
            predicates,
            settings,
            catalog,
        }
    }

    // explicit base, then the cached endpoint, then GITHUB_COPILOT_API_BASE, then the public API; no key is fetched
    pub async fn api_base_without_login(&mut self, api_base: Option<&str>) -> String {
        if let Some(explicit) = api_base {
            return explicit.to_owned();
        }
        if let Some(cached) = self.credentials.api_base().await {
            return cached;
        }
        self.settings
            .api_base
            .clone()
            .unwrap_or_else(|| String::from(DEFAULT_COPILOT_API_BASE))
    }

    // the supplied key is ignored: the Copilot key always comes from the authenticator
    pub async fn provider_info(
        &mut self,
        model: &str,
        supplied_api_base: Option<&str>,
        _supplied_api_key: Option<&str>,
        provider: &str,
    ) -> Result<ProviderInfo, CopilotChatError> {
        let api_base = self.api_base_without_login(supplied_api_base).await;
        let api_key = match self.credentials.api_key().await {
            Ok(key) => key,
            Err(source @ ApiKeyError::AccessToken(_)) => {
                return Err(CopilotChatError::Login(source));
            }
            Err(source) => {
                return Err(CopilotChatError::Authentication {
                    model: model.to_owned(),
                    provider: provider.to_owned(),
                    source,
                });
            }
        };
        Ok(ProviderInfo {
            api_base,
            api_key,
            provider: provider.to_owned(),
        })
    }

    // parent headers win over the Copilot defaults; a missing key only leaves the defaults out
    pub async fn validate_environment(
        &mut self,
        messages: &[CopilotMessage],
        request_id: Uuid,
    ) -> Result<Headers, CopilotChatError> {
        let mut headers = self.base.validate_environment();
        match self.credentials.api_key().await {
            Ok(key) => {
                // a cached record without a token renders as Bearer None, like the f-string in LiteLLM
                let token = key.unwrap_or_else(|| SecretString::from(String::from("None")));
                let mut defaults = copilot_default_headers(&token, Some(request_id));
                defaults.extend(headers);
                headers = defaults;
            }
            Err(source @ ApiKeyError::AccessToken(_)) => {
                return Err(CopilotChatError::Login(source));
            }
            // LiteLLM swallows GetAPIKeyError here and lets the request flow report it
            Err(
                ApiKeyError::Refresh
                | ApiKeyError::Save(_)
                | ApiKeyError::Encode(_)
                | ApiKeyError::MissingToken,
            ) => {}
        }
        headers.insert(
            String::from("X-Initiator"),
            String::from(initiator(messages)),
        );
        if has_vision_content(messages) {
            headers.insert(String::from("Copilot-Vision-Request"), String::from("true"));
        }
        Ok(headers)
    }

    // thinking and reasoning_effort join the list for a Claude name that the capability data marks as reasoning
    pub async fn supported_params(&mut self, model: &str) -> Vec<&'static str> {
        let mut params = self.base.supported_params(model, &self.catalog);
        let lowered = model.to_lowercase();
        if !lowered.contains("claude") {
            return params;
        }
        let request = CapabilityRequest {
            model: &lowered,
            custom_llm_provider: None,
        };
        if self
            .predicates
            .supports_factory(&request, NamedCapability::Reasoning.key())
            .await
        {
            for name in ["thinking", "reasoning_effort"] {
                if !params.contains(&name) {
                    params.push(name);
                }
            }
        }
        params
    }
}
