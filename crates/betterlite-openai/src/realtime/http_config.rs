use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use secrecy::{ExposeSecret as _, SecretString};

use crate::credentials::{
    ExplicitCredentials, LiteLlmGlobals, get_openai_credentials, resolve_openai_api_base,
};
use crate::openai_base_config::Headers;

pub const DEFAULT_API_BASE: &str = "https://api.openai.com";
const CLIENT_SECRETS_PATH: &str = "/v1/realtime/client_secrets";
const CALLS_PATH: &str = "/v1/realtime/calls";
const TRANSCRIPTION_SESSIONS_PATH: &str = "/v1/realtime/transcription_sessions";

// The globals and environment LiteLLM's OpenAIRealtimeHTTPConfig reads; OPENAI_BASE_URL is not one of them
pub struct RealtimeHttpSettings {
    pub litellm_api_base: Option<String>,
    pub litellm_api_key: Option<SecretString>,
    pub litellm_openai_key: Option<SecretString>,
    pub openai_api_base: Option<String>,
    pub openai_api_key: Option<SecretString>,
}

impl RealtimeHttpSettings {
    #[must_use]
    pub fn api_base(&self, api_base: Option<&str>) -> String {
        let env_api_base = self
            .openai_api_base
            .as_deref()
            .filter(|text| !text.is_empty())
            .unwrap_or(DEFAULT_API_BASE);
        let settings = OpenAiCredentialsSettings {
            api_base: env_api_base.to_owned(),
            organization: None,
        };
        resolve_openai_api_base(api_base, self.litellm_api_base.as_deref(), &settings)
    }

    #[must_use]
    pub fn api_key(&self, api_key: Option<&str>) -> SecretString {
        let credentials = get_openai_credentials(
            ExplicitCredentials {
                api_base: None,
                api_key: api_key.map(SecretString::from),
                organization: None,
            },
            LiteLlmGlobals {
                api_base: None,
                api_key: self.litellm_api_key.clone(),
                openai_key: self.litellm_openai_key.clone(),
                organization: None,
            },
            &OpenAiCredentialsSettings::default(),
            self.openai_api_key.clone(),
        );
        credentials.api_key.unwrap_or_default()
    }

    #[must_use]
    pub fn complete_url(&self, api_base: Option<&str>) -> String {
        let mut url = self.session_base(api_base);
        url.push_str(CLIENT_SECRETS_PATH);
        url
    }

    #[must_use]
    pub fn calls_url(&self, api_base: Option<&str>) -> String {
        let mut url = self.session_base(api_base);
        url.push_str(CALLS_PATH);
        url
    }

    #[must_use]
    pub fn transcription_session_url(&self, api_base: Option<&str>) -> String {
        let mut url = self.session_base(api_base);
        url.push_str(TRANSCRIPTION_SESSIONS_PATH);
        url
    }

    fn session_base(&self, api_base: Option<&str>) -> String {
        let base = self.api_base(api_base);
        let trimmed = base.trim_end_matches('/');
        trimmed.strip_suffix("/v1").unwrap_or(trimmed).to_owned()
    }
}

#[must_use]
pub fn validate_environment(headers: &Headers, api_key: Option<&SecretString>) -> Headers {
    let mut validated = headers.clone();
    let authorization = match api_key {
        Some(key) => format!("Bearer {}", key.expose_secret()),
        None => String::from("Bearer "),
    };
    validated.insert(String::from("Authorization"), authorization);
    validated.insert(
        String::from("Content-Type"),
        String::from("application/json"),
    );
    validated
}
