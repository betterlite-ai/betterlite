use std::future::Future;

use betterlite_config::sections::chatgpt_headers::Settings as HeaderSettings;
use secrecy::SecretString;
use serde_json::Value;

use super::headers::session::{LitellmParamsInput, SessionIdentity, ensure_chatgpt_session_id};
use super::headers::{PlatformEnvironment, get_chatgpt_default_headers};
use super::login::ChatGptLogin;
use super::login::DeviceDisplay;
use super::login::Pause;
use super::login::error::LoginError;
use super::login::http::AuthTransport;
use super::token_store::files::FileAccess;
use crate::openai_base_config::{CatalogCapabilities, Headers, Object, OpenAiConfig};

pub mod normalizer;
mod tests;

pub trait Authenticator {
    fn api_base(&self) -> &str;
    fn access_token(&mut self) -> impl Future<Output = Result<SecretString, LoginError>> + Send;
    fn account_id(&mut self) -> Option<String>;
}

impl<F, C, H, D> Authenticator for ChatGptLogin<F, C, H, D>
where
    F: FileAccess + Send,
    C: Pause + Send,
    H: AuthTransport + Send,
    D: DeviceDisplay + Send,
{
    fn api_base(&self) -> &str {
        &self.store.settings.api_base
    }

    fn access_token(&mut self) -> impl Future<Output = Result<SecretString, LoginError>> + Send {
        Self::get_access_token(self)
    }

    fn account_id(&mut self) -> Option<String> {
        self.store.get_account_id()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ChatGptError {
    #[error(
        "the access token for ChatGPT model {model} from provider {provider} could not be obtained"
    )]
    Authentication {
        model: String,
        provider: String,
        #[source]
        source: LoginError,
    },
    #[error("the ChatGPT login failed for model {model}")]
    Login {
        model: String,
        #[source]
        source: LoginError,
    },
}

#[derive(Debug, Clone)]
pub struct ProviderInfo {
    pub api_base: String,
    pub api_key: SecretString,
    pub provider: String,
}

pub struct ChatGptConfig<A> {
    pub base: OpenAiConfig,
    pub authenticator: A,
    pub header_settings: HeaderSettings,
}

impl<A: Authenticator> ChatGptConfig<A> {
    pub async fn provider_info(
        &mut self,
        model: &str,
        _supplied_api_base: Option<&str>,
        _supplied_api_key: Option<&str>,
        provider: &str,
    ) -> Result<ProviderInfo, ChatGptError> {
        let api_base = self.authenticator.api_base().to_owned();
        let api_key = self
            .authenticator
            .access_token()
            .await
            .map_err(|source| match source {
                LoginError::AccessToken(_) => ChatGptError::Authentication {
                    model: model.to_owned(),
                    provider: provider.to_owned(),
                    source,
                },
                LoginError::DeviceCode(_) | LoginError::Refresh(_) => ChatGptError::Login {
                    model: model.to_owned(),
                    source,
                },
            })?;
        Ok(ProviderInfo {
            api_base,
            api_key,
            provider: provider.to_owned(),
        })
    }

    #[must_use]
    pub fn validate_environment(
        &mut self,
        litellm_params: &LitellmParamsInput,
        platform: &dyn PlatformEnvironment,
        session: &dyn SessionIdentity,
    ) -> Headers {
        let validated = self.base.validate_environment();
        let account_id = self.authenticator.account_id();
        let session_id = ensure_chatgpt_session_id(litellm_params, session);
        let access_token = self.base.api_key.clone().unwrap_or_default();
        let mut headers = get_chatgpt_default_headers(
            &self.header_settings,
            platform,
            &access_token,
            account_id.as_deref(),
            Some(&session_id),
        );
        headers.extend(validated);
        headers
    }

    pub fn map_openai_params(
        &self,
        model: &str,
        non_default_params: &Object,
        optional_params: &mut Object,
        catalog: &CatalogCapabilities,
    ) {
        self.base
            .map_params(model, non_default_params, optional_params, catalog);
        optional_params
            .entry(String::from("stream"))
            .or_insert(Value::Bool(false));
    }
}
