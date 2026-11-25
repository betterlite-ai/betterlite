use super::convert::fields::{headers, text, timeout};
use super::convert::{ConvertError, DEFAULT_TIMEOUT, SettingsWire};
use super::error::{CallFailure, SessionError, convert_failure};
use crate::completion::setup::outbound_client;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::sections::openai_credentials::{
    DEFAULT_API_BASE, Settings as OpenAiCredentialSettings,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::credentials::{
    ExplicitCredentials, LiteLlmGlobals, OpenAiCredentials, get_openai_credentials,
};
use betterlite_openai::image_generation::logging::{
    ImageGenerationLogger, PostCallRecord, PreCallRecord,
};
use betterlite_openai::image_variations::logging::{
    ImageVariationLogger, PostCallRecord as VariationPostCall, PreCallRecord as VariationPreCall,
};
use betterlite_openai::openai_base_config::Headers;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_setup::RequestMap;
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// The process environment a session was built from, the outbound client and the redactor every failure goes through.
pub(crate) struct ImageSession {
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) client: ProxiedClient,
    pub(crate) redactor: Redactor,
}

impl ImageSession {
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
    fn credentials(
        &self,
        settings: &SettingsWire,
        explicit: ExplicitCredentials,
    ) -> OpenAiCredentials {
        let configured = OpenAiCredentialSettings {
            api_base: first_text(&self.environment, &["OPENAI_BASE_URL", "OPENAI_API_BASE"])
                .unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
            organization: first_text(&self.environment, &["OPENAI_ORGANIZATION"]),
        };
        let globals = LiteLlmGlobals {
            api_base: settings.api_base.clone(),
            api_key: settings.api_key.clone(),
            openai_key: settings.openai_key.clone(),
            organization: None,
        };
        let env_api_key =
            first_text(&self.environment, &["OPENAI_API_KEY"]).map(SecretString::from);
        get_openai_credentials(explicit, globals, &configured, env_api_key)
    }
}

// Everything a provider call shares: the caller's headers, its timeout and the resolved credentials.
pub(crate) struct CallSetup {
    pub(crate) headers: Headers,
    pub(crate) timeout: Duration,
    pub(crate) credentials: OpenAiCredentials,
}

pub(crate) fn call_setup(
    session: &ImageSession,
    arguments: &RequestMap,
    settings: &SettingsWire,
    model: &str,
    provider: &str,
) -> Result<CallSetup, CallFailure> {
    let failure =
        |error| convert_failure(&error, Some(model.to_owned()), Some(provider.to_owned()));
    let headers = headers(arguments).map_err(failure)?;
    let timeout = timeout(arguments).map_err(failure)?;
    let explicit = explicit_credentials(arguments).map_err(failure)?;
    Ok(CallSetup {
        headers,
        timeout: timeout.unwrap_or(DEFAULT_TIMEOUT),
        credentials: session.credentials(settings, explicit),
    })
}

pub(crate) fn explicit_credentials(
    arguments: &RequestMap,
) -> Result<ExplicitCredentials, ConvertError> {
    let api_base = text(arguments, "api_base")?.filter(|value| !value.is_empty());
    let api_key = text(arguments, "api_key")?.filter(|value| !value.is_empty());
    let organization = text(arguments, "organization")?.filter(|value| !value.is_empty());
    Ok(ExplicitCredentials {
        api_base,
        api_key: api_key.map(SecretString::from),
        organization,
    })
}

pub(crate) fn now_seconds() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_secs()).unwrap_or(i64::MAX),
        Err(_before_epoch) => 0,
    }
}

pub(crate) fn first_text(environment: &BTreeMap<String, String>, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        environment
            .get(*name)
            .filter(|value| !value.is_empty())
            .cloned()
    })
}

// Python logs the calls it runs in Rust through LiteLLM's callbacks, which never cross the boundary.
pub(crate) struct SilentImageLogger;

impl ImageGenerationLogger for SilentImageLogger {
    fn pre_call(&mut self, _record: PreCallRecord) -> Result<(), PrePostError> {
        Ok(())
    }

    fn post_call(&mut self, _record: PostCallRecord) {}
}

impl ImageVariationLogger for SilentImageLogger {
    fn pre_call(&mut self, _record: VariationPreCall) -> Result<(), PrePostError> {
        Ok(())
    }

    fn post_call(&mut self, _record: VariationPostCall) {}
}
