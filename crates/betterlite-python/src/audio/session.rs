use super::convert::{AudioSettings, text};
use super::error::{CallFailure, SessionError};
use crate::completion::setup::outbound_client;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::sections::openai_credentials::{
    DEFAULT_API_BASE, Settings as OpenAiCredentialSettings,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::credentials::{ExplicitCredentials, LiteLlmGlobals, get_openai_credentials};
use betterlite_openai::speech::logging::{SpeechLogRecord, SpeechLogger};
use betterlite_openai::transcription::logging::{
    TranscriptionLogger, TranscriptionPostCall, TranscriptionPreCall,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_setup::RequestMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::time::Duration;

// LiteLLM's constants.request_timeout: REQUEST_TIMEOUT or 6000, read once at import
const DEFAULT_TIMEOUT_SECONDS: f64 = 6000.0_f64;

// The process environment a session was built from, the outbound client and the redactor every failure goes through.
pub(crate) struct AudioSession {
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) client: ProxiedClient,
    pub(crate) redactor: Redactor,
}

pub(crate) struct Credentials {
    pub(crate) api_base: String,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) organization: Option<String>,
    pub(crate) project: Option<String>,
}

impl AudioSession {
    pub(crate) fn build(_settings: &str) -> Result<Self, SessionError> {
        let environment = process_environment();
        let client = outbound_client(&environment)?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
        Ok(Self {
            environment,
            client,
            redactor,
        })
    }

    // get_openai_credentials over this session's environment, the payload settings and the call
    pub(crate) fn credentials(
        &self,
        settings: &AudioSettings,
        arguments: &RequestMap,
    ) -> Result<Credentials, CallFailure> {
        let explicit = ExplicitCredentials {
            api_base: [text(arguments, "api_base")?, text(arguments, "base_url")?]
                .into_iter()
                .flatten()
                .find(|value| !value.is_empty()),
            api_key: text(arguments, "api_key")?.map(SecretString::from),
            organization: text(arguments, "organization")?,
        };
        let globals = LiteLlmGlobals {
            api_base: settings.api_base.clone(),
            api_key: settings.api_key.clone(),
            openai_key: settings.openai_key.clone(),
            organization: settings.organization.clone(),
        };
        let configured = OpenAiCredentialSettings {
            api_base: first_text(&self.environment, &["OPENAI_BASE_URL", "OPENAI_API_BASE"])
                .unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
            organization: first_text(&self.environment, &["OPENAI_ORGANIZATION"]),
        };
        let env_api_key =
            first_text(&self.environment, &["OPENAI_API_KEY"]).map(SecretString::from);
        let resolved = get_openai_credentials(explicit, globals, &configured, env_api_key);
        let project = [
            text(arguments, "project")?,
            settings.project.clone(),
            first_text(&self.environment, &["OPENAI_PROJECT"]),
        ]
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty());
        Ok(Credentials {
            api_base: resolved.api_base,
            api_key: resolved.api_key,
            organization: resolved.organization,
            project,
        })
    }

    // litellm.request_timeout: REQUEST_TIMEOUT, else the import-time 6000 seconds
    pub(crate) fn request_timeout(&self) -> Duration {
        let raw = self
            .environment
            .get("REQUEST_TIMEOUT")
            .cloned()
            .unwrap_or_default();
        let seconds = betterlite_env::number::<f64>("REQUEST_TIMEOUT", &raw)
            .ok()
            .flatten()
            .unwrap_or(DEFAULT_TIMEOUT_SECONDS);
        Duration::try_from_secs_f64(seconds).unwrap_or(Duration::from_secs(6000))
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

// Python logs the calls it runs in Rust through LiteLLM's callbacks, which never cross the boundary.
pub(crate) struct SilentAudioLogger;

impl TranscriptionLogger for SilentAudioLogger {
    fn pre_call(&mut self, _record: TranscriptionPreCall<'_>) -> Result<(), PrePostError> {
        Ok(())
    }

    fn response_headers(&mut self, _headers: &IndexMap<String, String>) {}

    fn post_call(&mut self, _record: TranscriptionPostCall<'_>) {}
}

impl SpeechLogger for SilentAudioLogger {
    fn pre_call(&mut self, _record: SpeechLogRecord<'_>) -> Result<(), PrePostError> {
        Ok(())
    }
}
