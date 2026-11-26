use betterlite_auth::auth_context_sources::sso::{
    SsoConfiguration, SsoRuntime, has_user_setup_sso, is_provider_configured,
};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use super::Input;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SsoScenario {
    microsoft_client_id: Option<String>,
    microsoft_client_secret: Option<String>,
    microsoft_tenant: Option<String>,
    google_client_id: Option<String>,
    google_client_secret: Option<String>,
    generic_client_id: Option<String>,
    generic_client_secret: Option<String>,
    generic_authorization_endpoint: Option<String>,
    generic_token_endpoint: Option<String>,
    generic_userinfo_endpoint: Option<String>,
    saml_idp_metadata_url: Option<String>,
    saml_idp_metadata_xml: Option<String>,
    saml_available: bool,
}

pub(super) fn scenario_answer(scenario: &SsoScenario) -> Value {
    let configuration = SsoConfiguration {
        microsoft_client_id: scenario.microsoft_client_id.clone(),
        google_client_id: scenario.google_client_id.clone(),
        generic_client_id: scenario.generic_client_id.clone(),
        saml_idp_metadata_url: scenario.saml_idp_metadata_url.clone(),
        saml_idp_metadata_xml: scenario.saml_idp_metadata_xml.clone(),
        google_client_secret: scenario
            .google_client_secret
            .clone()
            .map(SecretString::from),
        microsoft_client_secret: scenario
            .microsoft_client_secret
            .clone()
            .map(SecretString::from),
        microsoft_tenant: scenario.microsoft_tenant.clone(),
        generic_client_secret: scenario
            .generic_client_secret
            .clone()
            .map(SecretString::from),
        generic_authorization_endpoint: scenario.generic_authorization_endpoint.clone(),
        generic_token_endpoint: scenario.generic_token_endpoint.clone(),
        generic_userinfo_endpoint: scenario.generic_userinfo_endpoint.clone(),
    };
    let runtime = SsoRuntime {
        saml_available: scenario.saml_available,
    };
    json!({
        "discovery": has_user_setup_sso(&configuration),
        "ready": is_provider_configured(&configuration, runtime),
    })
}
