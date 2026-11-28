use std::io::Error as IoError;

use thiserror::Error;

use super::fetch::MetadataFetchError;
use super::idp_metadata::IdpMetadataError;
use super::settings::SettingsErrors;
use super::sp_metadata::SpMetadataErrors;

pub const UNAVAILABLE_DETAIL: &str = "SAML SSO requires the optional 'python3-saml' dependency, which is not installed. Re-install litellm with the saml extra: 'pip install litellm[saml]'. The saml extra bundles the native xmlsec/libxml2 libraries, so no system packages are required.";
pub const UNCONFIGURED_DETAIL: &str =
    "SAML SSO is not configured. Set SAML_IDP_METADATA_URL or SAML_IDP_METADATA_XML.";
pub const NO_USABLE_IDP_DETAIL: &str =
    "Could not parse an IdP entityID/SSO URL/certificate from the SAML metadata.";

#[derive(Debug, Error)]
pub enum SamlProviderError {
    #[error("the SAML toolkit backend is unavailable")]
    Unavailable,
    #[error("SAML SSO is not configured")]
    Unconfigured,
    #[error("the IdP metadata has no usable identity provider")]
    NoUsableIdp,
    #[error("Invalid SAML configuration: {0}")]
    InvalidSettings(#[source] SettingsErrors),
    #[error("invalid SP settings")]
    InvalidMetadataSettings(#[source] SettingsErrors),
    #[error("{0}")]
    InvalidMetadata(#[source] SpMetadataErrors),
    #[error("the IdP metadata could not be fetched")]
    Fetch(#[source] MetadataFetchError),
    #[error("the IdP metadata could not be parsed")]
    Parse(#[source] IdpMetadataError),
    #[error("the IdP redirect URL is not an http or https URL")]
    InvalidRedirectUrl,
    #[error("the AuthnRequest could not be compressed")]
    AuthnRequest(#[source] IoError),
    #[error("the clock is outside the supported range")]
    ClockOutOfRange,
}
