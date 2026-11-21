use betterlite_openai::openai_base_config::Headers;
use secrecy::{ExposeSecret as _, SecretString};

use super::headers::API_KEY_HEADER;

pub const DEFAULT_API_VERSION: &str = "2024-12-17";
pub const CONTENT_TYPE_HEADER: &str = "Content-Type";
pub const JSON_CONTENT_TYPE: &str = "application/json";
const CLIENT_SECRETS_PATH: &str = "/openai/realtime/client_secrets";
const CALLS_PATH: &str = "/openai/realtime/calls";
const TRANSCRIPTION_SESSIONS_PATH: &str = "/openai/realtime/transcription_sessions";

pub struct RealtimeHttpSettings {
    pub litellm_api_base: Option<String>,
    pub litellm_api_key: Option<SecretString>,
    pub azure_api_base: Option<String>,
    pub azure_api_key: Option<SecretString>,
    pub azure_api_version: Option<String>,
}

impl RealtimeHttpSettings {
    #[must_use]
    pub fn api_base(&self, api_base: Option<&str>) -> String {
        if let Some(value) = api_base {
            return value.to_owned();
        }
        if let Some(value) = &self.litellm_api_base {
            return value.clone();
        }
        self.azure_api_base
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or_default()
            .to_owned()
    }

    #[must_use]
    pub fn api_key(&self, api_key: Option<&str>) -> SecretString {
        if let Some(value) = api_key {
            return SecretString::from(value.to_owned());
        }
        if let Some(value) = &self.litellm_api_key {
            return value.clone();
        }
        self.azure_api_key
            .clone()
            .filter(|key| !key.expose_secret().is_empty())
            .unwrap_or_default()
    }

    #[must_use]
    pub fn complete_url(&self, api_base: Option<&str>, api_version: Option<&str>) -> String {
        format!(
            "{}{CLIENT_SECRETS_PATH}?api-version={}",
            self.session_base(api_base),
            self.api_version(api_version)
        )
    }

    #[must_use]
    pub fn calls_url(&self, api_base: Option<&str>, api_version: Option<&str>) -> String {
        format!(
            "{}{CALLS_PATH}?api-version={}",
            self.session_base(api_base),
            self.api_version(api_version)
        )
    }

    #[must_use]
    pub fn transcription_session_url(
        &self,
        api_base: Option<&str>,
        api_version: Option<&str>,
    ) -> String {
        format!(
            "{}{TRANSCRIPTION_SESSIONS_PATH}?api-version={}",
            self.session_base(api_base),
            self.api_version(api_version)
        )
    }

    fn session_base(&self, api_base: Option<&str>) -> String {
        self.api_base(api_base).trim_end_matches('/').to_owned()
    }

    fn api_version(&self, api_version: Option<&str>) -> String {
        if let Some(value) = api_version {
            return value.to_owned();
        }
        self.azure_api_version
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_API_VERSION)
            .to_owned()
    }
}

#[must_use]
pub fn validate_environment(headers: &Headers, api_key: Option<&SecretString>) -> Headers {
    let mut validated = headers.clone();
    let key = api_key.map_or("", |key| key.expose_secret());
    validated.insert(String::from(API_KEY_HEADER), key.to_owned());
    validated.insert(
        String::from(CONTENT_TYPE_HEADER),
        String::from(JSON_CONTENT_TYPE),
    );
    validated
}

#[must_use]
pub fn realtime_calls_headers(ephemeral_key: &str) -> Headers {
    Headers::from([(String::from(API_KEY_HEADER), ephemeral_key.to_owned())])
}
