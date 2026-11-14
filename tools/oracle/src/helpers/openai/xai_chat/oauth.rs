use std::future::Future;

use betterlite_config::sections::xai_identity::Settings as IdentitySettings;
use betterlite_openai::xai_chat::OauthTokens;
use betterlite_openai::xai_chat::params::ReasoningLookup;
use betterlite_openai::xai_tokens::error::XaiOauthError;
use secrecy::SecretString;

use super::Settings;

pub(super) struct Oauth {
    api_base: String,
    token: Option<&'static str>,
}

impl OauthTokens for Oauth {
    fn api_base(&self) -> String {
        self.api_base.clone()
    }

    fn access_token(&mut self) -> impl Future<Output = Result<SecretString, XaiOauthError>> + Send {
        let token = self.token;
        async move {
            token
                .map(|value| SecretString::from(String::from(value)))
                .ok_or(XaiOauthError::LoginRequired)
        }
    }
}

pub(super) struct CapabilityLookup {
    pub(super) outcome: Option<bool>,
}

impl ReasoningLookup<std::io::Error> for CapabilityLookup {
    fn supports_reasoning(
        &mut self,
        _model: &str,
    ) -> impl Future<Output = Result<bool, std::io::Error>> + Send {
        let outcome = self.outcome;
        async move { outcome.ok_or_else(|| std::io::Error::other("the capability lookup failed")) }
    }
}

pub(super) fn identity(settings: &Settings) -> IdentitySettings {
    IdentitySettings {
        xai_key: settings.xai_key.clone(),
        generic_api_key: None,
        environment_key: settings.environment_key.clone(),
        api_base: settings.api_base.clone(),
    }
}

pub(super) fn oauth(settings: &Settings, token: Option<&'static str>) -> Oauth {
    Oauth {
        api_base: settings
            .oauth_api_base
            .clone()
            .unwrap_or_else(|| String::from("https://api.x.ai/v1")),
        token,
    }
}
