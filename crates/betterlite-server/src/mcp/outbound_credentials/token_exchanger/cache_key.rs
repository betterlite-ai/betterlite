use betterlite_mcp::credential_contracts::auth_config::oauth::{
    TokenExchangeConfig, TokenExchangeProfile,
};
use betterlite_mcp::credential_contracts::token_endpoint_auth::TokenEndpointAuthMethod;
use secrecy::ExposeSecret as _;
use sha2::{Digest as _, Sha256};

// Everything that shapes the minted token is hashed into the key, so a config rotation re-exchanges before the TTL.
pub(super) fn cache_key(
    subject_token: &str,
    tenant_id: &str,
    config: &TokenExchangeConfig,
) -> String {
    let secret = config
        .client_secret
        .as_ref()
        .map_or("", |value| value.expose_secret());
    let profile = match config.profile {
        TokenExchangeProfile::Rfc8693 => "rfc8693",
        TokenExchangeProfile::EntraObo => "entra_obo",
    };
    let auth_method = config
        .token_endpoint_auth_method
        .map_or("", TokenEndpointAuthMethod::as_str);
    let material = [
        subject_token.to_owned(),
        tenant_id.to_owned(),
        profile.to_owned(),
        config.token_exchange_endpoint.clone().unwrap_or_default(),
        config.audience.clone().unwrap_or_default(),
        config.subject_token_type.clone(),
        config.client_id.clone().unwrap_or_default(),
        secret.to_owned(),
        auth_method.to_owned(),
        config.scopes.join(" "),
    ]
    .join("\u{0}");
    hex::encode(Sha256::digest(material.as_bytes()))
}
