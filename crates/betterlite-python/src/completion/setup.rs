mod tests;
use super::error::SessionError;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::{Config, GeneralSettings, LiteLlmSettings, ProxyTrust};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_openai::chat::deployment::OpenAiDefaults;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_wrapper::error::{CallError, plain_error};
use betterlite_sdk::completion_request::args::CompletionSettings;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings};
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::sync::Arc;
// litellm/constants.py's DEFAULT_REASONING_EFFORT_MEDIUM_THINKING_BUDGET
pub(crate) const DEFAULT_MEDIUM_THINKING_BUDGET: u32 = 2048;
pub(crate) fn redactor() -> Result<Redactor, CallError> {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|error| CallError::from(plain_error(&error.to_string())))
}
fn first_text(environment: &BTreeMap<String, String>, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        environment
            .get(*name)
            .filter(|value| !value.is_empty())
            .cloned()
    })
}
// main.py reads OPENAI_BASE_URL before OPENAI_API_BASE, and OPENAI_ORGANIZATION for the header
pub(crate) fn openai_defaults(environment: &BTreeMap<String, String>) -> OpenAiDefaults {
    OpenAiDefaults {
        request_timeout: None,
        api_bases: first_text(environment, &["OPENAI_BASE_URL", "OPENAI_API_BASE"])
            .into_iter()
            .collect(),
        api_keys: first_text(environment, &["OPENAI_API_KEY"])
            .map(SecretString::from)
            .into_iter()
            .collect(),
        organization: first_text(environment, &["OPENAI_ORGANIZATION"]),
        global_drop_params: false,
    }
}
// the outbound client honours the proxy variables of the environment the session was built from
pub(crate) fn outbound_client(
    environment: &BTreeMap<String, String>,
) -> Result<ProxiedClient, SessionError> {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), environment)
        .map_err(|source| SessionError::Proxy { source })?;
    ProxiedClient::outbound(&Arc::new(policy)).map_err(|source| SessionError::Client { source })
}
// the module-level credentials a session's settings carry, read the way main.py reads them
pub(crate) trait ModuleCredentials {
    fn api_key(&self) -> Option<SecretString>;
    fn api_base(&self) -> Option<String>;
}
impl ModuleCredentials for CompletionSettings {
    fn api_key(&self) -> Option<SecretString> {
        self.api_key.clone()
    }
    fn api_base(&self) -> Option<String> {
        self.api_base.clone()
    }
}
// litellm.api_key and litellm.api_base reach resolution as the module-level credentials
pub(crate) fn global_credentials<S: ModuleCredentials>(settings: &S) -> GlobalCredentials {
    GlobalCredentials {
        api_key: settings.api_key(),
        api_base: settings.api_base(),
        xai_key: None,
        inception_key: None,
        lemonade_key: None,
    }
}
// the tokenizer only reads sections.token_counting, and a Python session has no config file
pub(crate) fn runtime_config() -> Config {
    Config {
        model_list: Vec::new(),
        general_settings: Box::new(GeneralSettings {
            cipher: CipherSettings {
                signing_key: None,
                algorithm: Algorithm::XSalsa20Poly1305,
            },
            master_key: None,
        }),
        router_settings: Box::default(),
        litellm_settings: LiteLlmSettings {
            ssl_verify: None,
            ssl_certificate: None,
            http2: None,
            ssl_ecdh_curve: None,
            ssl_security_level: None,
            num_retries: None,
            request_timeout: None,
            drop_params: None,
            gemini_live_defer_setup: false,
            proxy_trust: ProxyTrust::default(),
            bedrock_metadata_fields: Vec::new(),
            modify_params: false,
            default_max_tokens: 0,
        },
        environment_variables: BTreeMap::new(),
        sections: Box::default(),
    }
}
