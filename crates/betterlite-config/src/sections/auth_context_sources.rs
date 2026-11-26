#[path = "auth_context_sources/tests.rs"]
mod tests;

use betterlite_env::number;
use secrecy::SecretString;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub const DEFAULT_MINIMUM_KEY_LENGTH: i64 = 16;

// None means the variable is unset; Some("") means it is set to an empty string, as os.getenv reads it.
#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {}

#[derive(Debug, Clone)]
pub struct Settings {
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
    pub minimum_custom_key_length: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            microsoft_client_id: None,
            google_client_id: None,
            generic_client_id: None,
            saml_idp_metadata_url: None,
            saml_idp_metadata_xml: None,
            google_client_secret: None,
            microsoft_client_secret: None,
            microsoft_tenant: None,
            generic_client_secret: None,
            generic_authorization_endpoint: None,
            generic_token_endpoint: None,
            generic_userinfo_endpoint: None,
            minimum_custom_key_length: DEFAULT_MINIMUM_KEY_LENGTH,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let minimum_custom_key_length = match lookup.lookup("MINIMUM_CUSTOM_KEY_LENGTH") {
            None => DEFAULT_MINIMUM_KEY_LENGTH,
            Some(text) => number::<i64>("MINIMUM_CUSTOM_KEY_LENGTH", &text)?
                .unwrap_or(DEFAULT_MINIMUM_KEY_LENGTH),
        };
        Ok(Settings {
            microsoft_client_id: lookup.lookup("MICROSOFT_CLIENT_ID"),
            google_client_id: lookup.lookup("GOOGLE_CLIENT_ID"),
            generic_client_id: lookup.lookup("GENERIC_CLIENT_ID"),
            saml_idp_metadata_url: lookup.lookup("SAML_IDP_METADATA_URL"),
            saml_idp_metadata_xml: lookup.lookup("SAML_IDP_METADATA_XML"),
            google_client_secret: lookup
                .lookup("GOOGLE_CLIENT_SECRET")
                .map(SecretString::from),
            microsoft_client_secret: lookup
                .lookup("MICROSOFT_CLIENT_SECRET")
                .map(SecretString::from),
            microsoft_tenant: lookup.lookup("MICROSOFT_TENANT"),
            generic_client_secret: lookup
                .lookup("GENERIC_CLIENT_SECRET")
                .map(SecretString::from),
            generic_authorization_endpoint: lookup.lookup("GENERIC_AUTHORIZATION_ENDPOINT"),
            generic_token_endpoint: lookup.lookup("GENERIC_TOKEN_ENDPOINT"),
            generic_userinfo_endpoint: lookup.lookup("GENERIC_USERINFO_ENDPOINT"),
            minimum_custom_key_length,
        })
    }
}
