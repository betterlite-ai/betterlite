use std::convert::Infallible;

use betterlite_azure::realtime::http_config::{
    RealtimeHttpSettings as AzureRealtimeSettings, realtime_calls_headers,
    validate_environment as validate_azure_environment,
};
use betterlite_config::api::realtime_http_calls::RealtimeHttpCallSettings;
use betterlite_context::resource_config_selection::realtime::{
    RealtimeHttpConfigTag, select_realtime_http_config,
};
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ProviderIdentity, ResolvedModel};
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::realtime::http_config::{
    DEFAULT_API_BASE, RealtimeHttpSettings as OpenAiRealtimeSettings,
    validate_environment as validate_openai_environment,
};
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::realtime_contracts::config::{BaseRealtimeHttpConfig, Environment, RealtimeUrl};
use crate::api::realtime_contracts::error::HeaderMap;

// The provider config ProviderConfigManager hands back, narrowed to the two providers that declare a realtime HTTP config.
pub enum RealtimeHttpConfig {
    OpenAi(OpenAiRealtimeSettings),
    Azure(AzureRealtimeSettings),
}

impl BaseRealtimeHttpConfig for RealtimeHttpConfig {
    type Error = Infallible;

    fn get_api_base(&self, api_base: Option<&str>) -> Result<String, Infallible> {
        Ok(match self {
            Self::OpenAi(settings) => settings.api_base(api_base),
            Self::Azure(settings) => settings.api_base(api_base),
        })
    }

    fn get_api_key(&self, api_key: Option<&str>) -> Result<String, Infallible> {
        Ok(match self {
            Self::OpenAi(settings) => settings.api_key(api_key).expose_secret().to_owned(),
            Self::Azure(settings) => settings.api_key(api_key).expose_secret().to_owned(),
        })
    }

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        _model: &str,
        api_version: Option<&str>,
    ) -> String {
        match self {
            Self::OpenAi(settings) => settings.complete_url(api_base),
            Self::Azure(settings) => settings.complete_url(api_base, api_version),
        }
    }

    fn validate_environment(&self, environment: &Environment<'_>) -> Result<HeaderMap, Infallible> {
        let headers = text_headers(environment.headers);
        let api_key = environment.api_key.map(SecretString::from);
        let validated = match self {
            Self::OpenAi(_) => validate_openai_environment(&headers, api_key.as_ref()),
            Self::Azure(_) => validate_azure_environment(&headers, api_key.as_ref()),
        };
        Ok(json_headers(&validated))
    }

    fn get_transcription_session_url(&self, url: &RealtimeUrl<'_>) -> String {
        match self {
            Self::OpenAi(settings) => settings.transcription_session_url(url.api_base),
            Self::Azure(settings) => {
                settings.transcription_session_url(url.api_base, url.api_version)
            }
        }
    }

    fn get_realtime_calls_url(&self, url: &RealtimeUrl<'_>) -> String {
        match self {
            Self::OpenAi(settings) => settings.calls_url(url.api_base),
            Self::Azure(settings) => settings.calls_url(url.api_base, url.api_version),
        }
    }

    fn get_realtime_calls_headers(&self, ephemeral_key: &str) -> HeaderMap {
        match self {
            Self::OpenAi(_) => HeaderMap::from([(
                String::from("Authorization"),
                Json::Text(format!("Bearer {ephemeral_key}")),
            )]),
            Self::Azure(_) => json_headers(&realtime_calls_headers(ephemeral_key)),
        }
    }
}

// The selection: only a provider LiteLLM lists reaches ProviderConfigManager, and only B157's two tags have a config.
#[must_use]
pub fn provider_config(
    resolved: &ResolvedModel,
    settings: &RealtimeHttpCallSettings,
) -> Option<RealtimeHttpConfig> {
    let ProviderIdentity::Known(provider) = &resolved.provider else {
        return None;
    };
    Some(match select_realtime_http_config("", *provider)? {
        RealtimeHttpConfigTag::OpenAI => RealtimeHttpConfig::OpenAi(OpenAiRealtimeSettings {
            litellm_api_base: settings.api_base.clone(),
            litellm_api_key: settings.api_key.clone(),
            litellm_openai_key: settings.openai_key.clone(),
            openai_api_base: settings.openai_api_base.clone(),
            openai_api_key: settings.openai_api_key.clone(),
        }),
        RealtimeHttpConfigTag::Azure => RealtimeHttpConfig::Azure(AzureRealtimeSettings {
            litellm_api_base: settings.api_base.clone(),
            litellm_api_key: settings.api_key.clone(),
            azure_api_base: settings.azure_api_base.clone(),
            azure_api_key: settings.azure_api_key.clone(),
            azure_api_version: settings.azure_api_version.clone(),
        }),
    })
}

// _get_realtime_http_provider_config's return: the config, the base with its trailing slashes gone, and the key.
pub struct RealtimeHttpCredentials {
    pub config: Option<RealtimeHttpConfig>,
    pub api_base: String,
    pub api_key: String,
}

#[must_use]
pub fn credentials(
    resolved: &ResolvedModel,
    settings: &RealtimeHttpCallSettings,
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
) -> RealtimeHttpCredentials {
    let config = provider_config(resolved, settings);
    let raw_api_base = resolved.api_base.as_deref().or(api_base);
    let raw_api_key = resolved.api_key.as_ref().or(api_key);
    let (base, key) = config.as_ref().map_or_else(
        || {
            (
                raw_api_base
                    .or(settings.api_base.as_deref())
                    .unwrap_or(DEFAULT_API_BASE)
                    .to_owned(),
                raw_api_key
                    .or(settings.api_key.as_ref())
                    .or(settings.openai_key.as_ref())
                    .or(settings.openai_api_key.as_ref())
                    .map(|secret| secret.expose_secret().to_owned())
                    .unwrap_or_default(),
            )
        },
        |selected| {
            (
                selected.get_api_base(raw_api_base).unwrap_or_default(),
                selected
                    .get_api_key(raw_api_key.map(SecretString::expose_secret))
                    .unwrap_or_default(),
            )
        },
    );
    RealtimeHttpCredentials {
        config,
        api_base: base.trim_end_matches('/').to_owned(),
        api_key: key,
    }
}

// HTTP header values are text; the realtime HTTP handlers pass no inbound headers at all.
fn text_headers(headers: &HeaderMap) -> Headers {
    headers
        .iter()
        .filter_map(|(name, value)| match value {
            Json::Text(text) => Some((name.clone(), text.clone())),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        })
        .collect()
}

fn json_headers(headers: &Headers) -> HeaderMap {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}
