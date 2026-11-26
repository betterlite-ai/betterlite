use secrecy::{ExposeSecret as _, SecretString};

use betterlite_config::sections::auth_context_sources::Settings;

// B063 installs the SAML runtime; until a route owner reports it, metadata alone is not readiness.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SsoRuntime {
    pub saml_available: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SsoConfiguration {
    pub microsoft_client_id: Option<String>,
    pub google_client_id: Option<String>,
    pub generic_client_id: Option<String>,
    pub saml_idp_metadata_url: Option<String>,
    pub saml_idp_metadata_xml: Option<String>,
    pub google_client_secret: Option<SecretString>,
    pub microsoft_client_secret: Option<SecretString>,
    pub microsoft_tenant: Option<String>,
    pub generic_client_secret: Option<SecretString>,
    pub generic_authorization_endpoint: Option<String>,
    pub generic_token_endpoint: Option<String>,
    pub generic_userinfo_endpoint: Option<String>,
}

impl From<&Settings> for SsoConfiguration {
    fn from(settings: &Settings) -> Self {
        Self {
            microsoft_client_id: settings.microsoft_client_id.clone(),
            google_client_id: settings.google_client_id.clone(),
            generic_client_id: settings.generic_client_id.clone(),
            saml_idp_metadata_url: settings.saml_idp_metadata_url.clone(),
            saml_idp_metadata_xml: settings.saml_idp_metadata_xml.clone(),
            google_client_secret: settings.google_client_secret.clone(),
            microsoft_client_secret: settings.microsoft_client_secret.clone(),
            microsoft_tenant: settings.microsoft_tenant.clone(),
            generic_client_secret: settings.generic_client_secret.clone(),
            generic_authorization_endpoint: settings.generic_authorization_endpoint.clone(),
            generic_token_endpoint: settings.generic_token_endpoint.clone(),
            generic_userinfo_endpoint: settings.generic_userinfo_endpoint.clone(),
        }
    }
}

// has_user_setup_sso: presence, not readiness, because the UI only decides whether to show the button.
#[must_use]
pub fn has_user_setup_sso(configuration: &SsoConfiguration) -> bool {
    configuration.microsoft_client_id.is_some()
        || configuration.google_client_id.is_some()
        || configuration.generic_client_id.is_some()
        || configuration
            .saml_idp_metadata_url
            .as_deref()
            .is_some_and(|url| !url.is_empty())
        || configuration
            .saml_idp_metadata_xml
            .as_deref()
            .is_some_and(|xml| !xml.is_empty())
}

// Every provider is checked on its own, so an incomplete one can never mask a complete one.
#[must_use]
pub fn is_sso_provider_fully_configured(
    configuration: &SsoConfiguration,
    runtime: SsoRuntime,
) -> bool {
    is_google_ready(configuration)
        || is_microsoft_ready(configuration)
        || is_generic_oauth_ready(configuration)
        || is_saml_ready(configuration, runtime)
}

// _is_google_ready: a client id without its secret cannot complete a sign-in.
fn is_google_ready(configuration: &SsoConfiguration) -> bool {
    configuration
        .google_client_id
        .as_deref()
        .is_some_and(|id| !id.is_empty())
        && configuration
            .google_client_secret
            .as_ref()
            .is_some_and(|secret| !secret.expose_secret().is_empty())
}

// _is_microsoft_ready: the tenant is as required as the secret.
fn is_microsoft_ready(configuration: &SsoConfiguration) -> bool {
    configuration
        .microsoft_client_id
        .as_deref()
        .is_some_and(|id| !id.is_empty())
        && configuration
            .microsoft_client_secret
            .as_ref()
            .is_some_and(|secret| !secret.expose_secret().is_empty())
        && configuration
            .microsoft_tenant
            .as_deref()
            .is_some_and(|tenant| !tenant.is_empty())
}

// _is_generic_oauth_ready: all three endpoints join the id and secret.
fn is_generic_oauth_ready(configuration: &SsoConfiguration) -> bool {
    configuration
        .generic_client_id
        .as_deref()
        .is_some_and(|id| !id.is_empty())
        && configuration
            .generic_client_secret
            .as_ref()
            .is_some_and(|secret| !secret.expose_secret().is_empty())
        && configuration
            .generic_authorization_endpoint
            .as_deref()
            .is_some_and(|endpoint| !endpoint.is_empty())
        && configuration
            .generic_token_endpoint
            .as_deref()
            .is_some_and(|endpoint| !endpoint.is_empty())
        && configuration
            .generic_userinfo_endpoint
            .as_deref()
            .is_some_and(|endpoint| !endpoint.is_empty())
}

// _is_saml_ready: metadata alone is not ready without the runtime B063 installs.
fn is_saml_ready(configuration: &SsoConfiguration, runtime: SsoRuntime) -> bool {
    let metadata = configuration
        .saml_idp_metadata_url
        .as_deref()
        .is_some_and(|url| !url.is_empty())
        || configuration
            .saml_idp_metadata_xml
            .as_deref()
            .is_some_and(|xml| !xml.is_empty());
    metadata && runtime.saml_available
}

// abbreviate_api_key: the last four characters are enough to match a key in a log, and nothing more.
#[must_use]
pub fn abbreviate_api_key(api_key: &SecretString, minimum_length: i64) -> String {
    let text = api_key.expose_secret();
    let characters = text.chars().count();
    let minimum = usize::try_from(minimum_length).unwrap_or(0);
    if characters < minimum {
        return "sk-...".to_owned();
    }
    let tail: String = text.chars().skip(characters.saturating_sub(4)).collect();
    format!("sk-...{tail}")
}
