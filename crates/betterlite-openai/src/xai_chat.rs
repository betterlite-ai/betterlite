pub mod error;
pub mod params;
pub mod request;
pub mod response;
pub mod stream;

mod tests;

use std::future::Future;

use betterlite_config::sections::xai_identity::Settings as IdentitySettings;
use secrecy::{ExposeSecret as _, SecretString};

use crate::chat_choices::endpoint;
use crate::openai_base_config::Headers;
use crate::xai_identity::{KeyOrder, get_api_key};
use crate::xai_tokens::error::XaiOauthError;
use error::XaiChatError;

pub const PROVIDER: &str = "xai";
pub const DEFAULT_API_BASE: &str = "https://api.x.ai/v1";

// B1400's authenticator is async because it may refresh an expired token; the composing layer implements this seam
pub trait OauthTokens {
    fn api_base(&self) -> String;
    fn access_token(&mut self) -> impl Future<Output = Result<SecretString, XaiOauthError>> + Send;
}

pub struct ProviderInfo {
    pub api_base: String,
    pub api_key: Option<SecretString>,
}

#[must_use]
pub fn provider_info(
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    settings: &IdentitySettings,
) -> ProviderInfo {
    let api_base = api_base
        .or(settings.api_base.as_deref())
        .unwrap_or(DEFAULT_API_BASE)
        .to_owned();
    ProviderInfo {
        api_base,
        api_key: get_api_key(api_key, KeyOrder::Chat, settings),
    }
}

pub struct Environment<'request, A> {
    pub headers: Headers,
    pub api_key: Option<&'request SecretString>,
    pub settings: &'request IdentitySettings,
    pub model: &'request str,
    pub use_xai_oauth: bool,
    pub oauth: &'request mut A,
}

// XAIChatConfig.validate_environment: only a truthy use_xai_oauth without a resolved key reaches B1400
pub async fn validate_environment<A>(
    environment: Environment<'_, A>,
) -> Result<Headers, XaiChatError>
where
    A: OauthTokens,
{
    let key = get_api_key(environment.api_key, KeyOrder::Chat, environment.settings);
    let mut headers = environment.headers;
    if !environment.use_xai_oauth || key.is_some() {
        return Ok(endpoint::validate_environment(key.as_ref(), &headers));
    }
    let token =
        environment
            .oauth
            .access_token()
            .await
            .map_err(|source| XaiChatError::Authentication {
                model: environment.model.to_owned(),
                source,
            })?;
    headers.insert(
        String::from("Authorization"),
        format!("Bearer {}", token.expose_secret()),
    );
    if !headers.contains_key("content-type") && !headers.contains_key("Content-Type") {
        headers.insert(
            String::from("Content-Type"),
            String::from("application/json"),
        );
    }
    Ok(headers)
}

pub struct UrlInput<'request, A> {
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub settings: &'request IdentitySettings,
    pub use_xai_oauth: bool,
    pub oauth: &'request A,
}

// XAIChatConfig.get_complete_url: OAuth swaps the base only when no key resolves
#[must_use]
pub fn complete_url<A>(input: UrlInput<'_, A>) -> String
where
    A: OauthTokens,
{
    let resolved = provider_info(input.api_base, input.api_key, input.settings);
    let base = if input.use_xai_oauth && resolved.api_key.is_none() {
        input.oauth.api_base()
    } else {
        resolved.api_base
    };
    endpoint::complete_url(Some(&base))
}
