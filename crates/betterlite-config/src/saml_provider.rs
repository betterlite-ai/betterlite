use betterlite_env::EnvError;

use crate::loader::EnvironmentLookup;
use crate::sso_identity_bridge::{SAML_IDP_METADATA_URL_ENV, SAML_IDP_METADATA_XML_ENV};

mod tests;

pub const SAML_SP_ENTITY_ID_ENV: &str = "SAML_SP_ENTITY_ID";
pub const SAML_SP_NAME_ID_FORMAT_ENV: &str = "SAML_SP_NAME_ID_FORMAT";
pub const SAML_STRICT_ENV: &str = "SAML_STRICT";
pub const SAML_ASSERTIONS_SIGNED_ENV: &str = "SAML_WANT_ASSERTIONS_SIGNED";
pub const SAML_WANT_MESSAGES_SIGNED_ENV: &str = "SAML_WANT_MESSAGES_SIGNED";
pub const SAML_AUTHN_REQUESTS_SIGNED_ENV: &str = "SAML_AUTHN_REQUESTS_SIGNED";
pub const SAML_VALIDATE_CERT_ENV: &str = "SAML_IDP_METADATA_VALIDATE_CERT";

pub const DEFAULT_NAME_ID_FORMAT: &str = "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress";

// clippy's struct_excessive_bools allows two bools per struct, and these switches are independent
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SamlFlag(pub bool);

// every SAML value LiteLLM reads with os.getenv, with absent kept apart from an empty string
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SamlProviderSettings {
    pub idp_metadata_url: Option<String>,
    pub idp_metadata_xml: Option<String>,
    pub sp_entity_id: Option<String>,
    pub sp_name_id_format: Option<String>,
    pub strict: SamlFlag,
    pub want_assertions_signed: SamlFlag,
    pub want_messages_signed: SamlFlag,
    pub authn_requests_signed: SamlFlag,
    pub idp_metadata_validate_cert: SamlFlag,
}

impl SamlProviderSettings {
    // SAMLAuthHandler.is_saml_configured: bool(metadata_url or metadata_xml), so empty is not configured
    #[must_use]
    pub fn is_configured(&self) -> bool {
        let url = self
            .idp_metadata_url
            .as_deref()
            .is_some_and(|value| !value.is_empty());
        let xml = self
            .idp_metadata_xml
            .as_deref()
            .is_some_and(|value| !value.is_empty());
        url || xml
    }
}

fn env_flag<L>(lookup: &L, name: &'static str, default: bool) -> Result<bool, EnvError>
where
    L: EnvironmentLookup,
{
    betterlite_env::flag(name, &lookup.lookup(name).unwrap_or_default())
        .map(|value| value.unwrap_or(default))
}

pub fn load_saml_provider_settings<L>(lookup: &L) -> Result<SamlProviderSettings, EnvError>
where
    L: EnvironmentLookup,
{
    Ok(SamlProviderSettings {
        idp_metadata_url: lookup.lookup(SAML_IDP_METADATA_URL_ENV),
        idp_metadata_xml: lookup.lookup(SAML_IDP_METADATA_XML_ENV),
        sp_entity_id: lookup.lookup(SAML_SP_ENTITY_ID_ENV),
        sp_name_id_format: lookup.lookup(SAML_SP_NAME_ID_FORMAT_ENV),
        strict: SamlFlag(env_flag(lookup, SAML_STRICT_ENV, true)?),
        want_assertions_signed: SamlFlag(env_flag(lookup, SAML_ASSERTIONS_SIGNED_ENV, true)?),
        want_messages_signed: SamlFlag(env_flag(lookup, SAML_WANT_MESSAGES_SIGNED_ENV, false)?),
        authn_requests_signed: SamlFlag(env_flag(lookup, SAML_AUTHN_REQUESTS_SIGNED_ENV, false)?),
        idp_metadata_validate_cert: SamlFlag(env_flag(lookup, SAML_VALIDATE_CERT_ENV, true)?),
    })
}
