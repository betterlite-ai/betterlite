use betterlite_config::sso_identity_bridge::SsoIdentityBridgeSettings;

const GENERIC_OIDC_REMEDY: &str = "Point SSO at the generic OIDC provider (GENERIC_CLIENT_ID), the one login path whose token response the gateway retains an id_token from";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveSsoProvider {
    Google,
    Microsoft,
    Generic,
    Saml,
    None,
}

impl ActiveSsoProvider {
    #[must_use]
    pub const fn value(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Microsoft => "microsoft",
            Self::Generic => "generic",
            Self::Saml => "saml",
            Self::None => "none",
        }
    }
}

// the SSO callback dispatches to the first configured branch, not to everything configured
#[must_use]
pub const fn active_sso_provider(settings: &SsoIdentityBridgeSettings) -> ActiveSsoProvider {
    if settings.google_client_id.is_some() {
        return ActiveSsoProvider::Google;
    }
    if settings.microsoft_client_id.is_some() {
        return ActiveSsoProvider::Microsoft;
    }
    if settings.generic_client_id.is_some() {
        return ActiveSsoProvider::Generic;
    }
    if settings.saml_idp_metadata_url.is_some() || settings.saml_idp_metadata_xml.is_some() {
        return ActiveSsoProvider::Saml;
    }
    ActiveSsoProvider::None
}

#[must_use]
pub fn id_jag_assertion_capture_gap(settings: &SsoIdentityBridgeSettings) -> Option<String> {
    let provider = active_sso_provider(settings);
    match provider {
        ActiveSsoProvider::Generic => None,
        ActiveSsoProvider::None => Some(format!(
            "no SSO provider is configured, so no IdP identity assertion is ever captured and ID-JAG credential resolution fails for every user. {GENERIC_OIDC_REMEDY}"
        )),
        ActiveSsoProvider::Google | ActiveSsoProvider::Microsoft | ActiveSsoProvider::Saml => {
            Some(format!(
                "the active SSO provider ({}) has no identity-assertion capture path, so no IdP id_token is ever stored and ID-JAG credential resolution fails for every user no matter how often they sign in. {GENERIC_OIDC_REMEDY}",
                provider.value()
            ))
        }
    }
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn id_jag_assertion_capture_gap_at_startup(
    settings: &SsoIdentityBridgeSettings,
) -> Option<String> {
    // config load runs before the database's SSO settings are reconciled, so an unresolved provider is not a gap yet
    if active_sso_provider(settings) == ActiveSsoProvider::None {
        return None;
    }
    id_jag_assertion_capture_gap(settings)
}
