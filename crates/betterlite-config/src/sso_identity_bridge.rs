use crate::loader::EnvironmentLookup;

mod tests;

pub const GOOGLE_CLIENT_ID_ENV: &str = "GOOGLE_CLIENT_ID";
pub const MICROSOFT_CLIENT_ID_ENV: &str = "MICROSOFT_CLIENT_ID";
pub const GENERIC_CLIENT_ID_ENV: &str = "GENERIC_CLIENT_ID";
pub const SAML_IDP_METADATA_URL_ENV: &str = "SAML_IDP_METADATA_URL";
pub const SAML_IDP_METADATA_XML_ENV: &str = "SAML_IDP_METADATA_XML";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SsoIdentityBridgeSettings {
    pub google_client_id: Option<String>,
    pub microsoft_client_id: Option<String>,
    pub generic_client_id: Option<String>,
    pub saml_idp_metadata_url: Option<String>,
    pub saml_idp_metadata_xml: Option<String>,
}

#[must_use]
pub fn load_bridge_settings<L>(lookup: &L) -> SsoIdentityBridgeSettings
where
    L: EnvironmentLookup,
{
    // the callback selects a branch on presence, so an empty client ID still wins over the next one
    SsoIdentityBridgeSettings {
        google_client_id: lookup.lookup(GOOGLE_CLIENT_ID_ENV),
        microsoft_client_id: lookup.lookup(MICROSOFT_CLIENT_ID_ENV),
        generic_client_id: lookup.lookup(GENERIC_CLIENT_ID_ENV),
        // is_saml_configured tests the metadata values for truth, so an empty one is not configured
        saml_idp_metadata_url: lookup
            .lookup(SAML_IDP_METADATA_URL_ENV)
            .filter(|value| !value.is_empty()),
        saml_idp_metadata_xml: lookup
            .lookup(SAML_IDP_METADATA_XML_ENV)
            .filter(|value| !value.is_empty()),
    }
}
