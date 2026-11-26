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

// One setting counts when it is present and not empty, the rule every provider shares.
fn filled(value: Option<&str>) -> bool {
    match value {
        Some(text) => !text.is_empty(),
        None => false,
    }
}

// has_user_setup_sso: presence, not readiness, because the UI only decides whether to show the button.
#[must_use]
pub fn has_user_setup_sso(configuration: &SsoConfiguration) -> bool {
    configuration.microsoft_client_id.is_some()
        || configuration.google_client_id.is_some()
        || configuration.generic_client_id.is_some()
        || filled(configuration.saml_idp_metadata_url.as_deref())
        || filled(configuration.saml_idp_metadata_xml.as_deref())
}

// Each provider is checked on its own, so an incomplete one can never mask a complete one.
#[must_use]
pub fn is_provider_configured(configuration: &SsoConfiguration, runtime: SsoRuntime) -> bool {
    let google = filled(configuration.google_client_id.as_deref())
        && configuration
            .google_client_secret
            .as_ref()
            .is_some_and(|secret| !secret.expose_secret().is_empty());
    let microsoft = filled(configuration.microsoft_client_id.as_deref())
        && configuration
            .microsoft_client_secret
            .as_ref()
            .is_some_and(|secret| !secret.expose_secret().is_empty())
        && filled(configuration.microsoft_tenant.as_deref());
    let generic = filled(configuration.generic_client_id.as_deref())
        && configuration
            .generic_client_secret
            .as_ref()
            .is_some_and(|secret| !secret.expose_secret().is_empty())
        && filled(configuration.generic_authorization_endpoint.as_deref())
        && filled(configuration.generic_token_endpoint.as_deref())
        && filled(configuration.generic_userinfo_endpoint.as_deref());
    let saml = runtime.saml_available
        && (filled(configuration.saml_idp_metadata_url.as_deref())
            || filled(configuration.saml_idp_metadata_xml.as_deref()));
    google || microsoft || generic || saml
}

// Abbreviation is for compatible response parameters; it does not permit logging credentials.
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
