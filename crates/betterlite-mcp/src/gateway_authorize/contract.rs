mod tests;

use serde::Serialize;

use crate::gateway_dcr::contracts::supported_grant_types;

pub const CONTRACT_VERSION: u8 = 1;

#[derive(Debug, Serialize)]
pub struct NativeClientAuthContract {
    pub contract_version: u8,
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: String,
    pub revocation_endpoint: String,
    pub resource: String,
    pub response_types_supported: [&'static str; 1],
    pub grant_types_supported: Vec<&'static str>,
    #[serde(rename = "code_challenge_methods_supported")]
    pub challenge_methods: [&'static str; 1],
    #[serde(rename = "token_endpoint_auth_methods_supported")]
    pub token_auth_methods: [&'static str; 1],
    #[serde(rename = "revocation_endpoint_auth_methods_supported")]
    pub revocation_auth_methods: [&'static str; 1],
}

#[must_use]
pub fn native_client_auth_contract(
    base_url: &str,
    token_exchange_available: bool,
) -> NativeClientAuthContract {
    NativeClientAuthContract {
        contract_version: CONTRACT_VERSION,
        issuer: base_url.to_owned(),
        authorization_endpoint: format!("{base_url}/authorize"),
        token_endpoint: format!("{base_url}/token"),
        registration_endpoint: format!("{base_url}/register"),
        revocation_endpoint: format!("{base_url}/revoke"),
        resource: base_url.to_owned(),
        response_types_supported: ["code"],
        grant_types_supported: supported_grant_types(token_exchange_available),
        challenge_methods: ["S256"],
        token_auth_methods: ["none"],
        revocation_auth_methods: ["none"],
    }
}
