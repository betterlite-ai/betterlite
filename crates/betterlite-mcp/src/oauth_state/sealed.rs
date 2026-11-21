mod tests;

use betterlite_db::mcp_stored_contracts::MCPTokenEndpointAuthMethod;
use betterlite_db::mcp_stored_contracts::secret::StoredSecret;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::SealCipher;

pub const BRIDGE_CODE_PREFIX: &str = "llm_bcode_";
pub const PASSTHROUGH_CODE_PREFIX: &str = "llm_ptcode_";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BridgeAuthorizationCode {
    pub litellm_user_id: String,
    pub mcp_server_id: String,
    pub oauth_nonce: Option<String>,
    pub upstream_code: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PassthroughAuthorizationCode {
    pub client_id: String,
    pub client_secret: Option<StoredSecret>,
    pub mcp_server_id: String,
    pub token_endpoint_auth_method: Option<MCPTokenEndpointAuthMethod>,
    pub upstream_code: String,
}

#[derive(Clone, Copy, Debug)]
pub struct PassthroughClient<'code> {
    pub client_id: &'code str,
    pub client_secret: Option<&'code str>,
    pub token_endpoint_auth_method: Option<MCPTokenEndpointAuthMethod>,
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum RedeemError {
    #[error("Authorization code was issued for a different MCP server")]
    WrongServer,
    #[error("code_verifier is required to redeem this authorization code")]
    VerifierRequired,
}

pub trait BridgeCodePrefix {
    fn is_bridge_code(&self) -> bool;
}

impl BridgeCodePrefix for str {
    fn is_bridge_code(&self) -> bool {
        self.starts_with(BRIDGE_CODE_PREFIX)
    }
}

#[must_use]
pub fn seal_bridge_code(
    upstream_code: &str,
    litellm_user_id: &str,
    mcp_server_id: &str,
    oauth_nonce: Option<&str>,
    cipher: &dyn SealCipher,
) -> String {
    let sealed = BridgeAuthorizationCode {
        litellm_user_id: litellm_user_id.to_owned(),
        mcp_server_id: mcp_server_id.to_owned(),
        oauth_nonce: oauth_nonce.map(str::to_owned),
        upstream_code: upstream_code.to_owned(),
    };
    let json = match serde_json::to_string(&sealed) {
        Ok(json) => json,
        Err(_error) => String::new(),
    };
    format!("{BRIDGE_CODE_PREFIX}{}", cipher.encrypt(&json))
}

#[must_use]
pub fn open_bridge_code(code: &str, cipher: &dyn SealCipher) -> Option<BridgeAuthorizationCode> {
    let sealed = code.strip_prefix(BRIDGE_CODE_PREFIX)?;
    let json = cipher.decrypt(sealed, "bridge_authorization_code")?;
    let opened: BridgeAuthorizationCode = serde_json::from_str(&json).ok()?;
    if opened.litellm_user_id.is_empty()
        || opened.mcp_server_id.is_empty()
        || opened.upstream_code.is_empty()
    {
        return None;
    }
    Some(opened)
}

#[must_use]
pub fn seal_passthrough_code(
    upstream_code: &str,
    client: &PassthroughClient<'_>,
    mcp_server_id: &str,
    cipher: &dyn SealCipher,
) -> String {
    let sealed = PassthroughAuthorizationCode {
        client_id: client.client_id.to_owned(),
        client_secret: client
            .client_secret
            .map(|secret| StoredSecret(SecretString::from(secret.to_owned()))),
        mcp_server_id: mcp_server_id.to_owned(),
        token_endpoint_auth_method: client.token_endpoint_auth_method,
        upstream_code: upstream_code.to_owned(),
    };
    let json = match serde_json::to_string(&sealed) {
        Ok(json) => json,
        Err(_error) => String::new(),
    };
    format!("{PASSTHROUGH_CODE_PREFIX}{}", cipher.encrypt(&json))
}

#[must_use]
pub fn open_passthrough_code(
    code: &str,
    cipher: &dyn SealCipher,
) -> Option<PassthroughAuthorizationCode> {
    let sealed = code.strip_prefix(PASSTHROUGH_CODE_PREFIX)?;
    let json = cipher.decrypt(sealed, "passthrough_authorization_code")?;
    let opened: PassthroughAuthorizationCode = serde_json::from_str(&json).ok()?;
    if opened.client_id.is_empty()
        || opened.mcp_server_id.is_empty()
        || opened.upstream_code.is_empty()
    {
        return None;
    }
    Some(opened)
}

pub fn redeem_passthrough_code(
    code: Option<&str>,
    server_id: &str,
    code_verifier: Option<&str>,
    cipher: &dyn SealCipher,
) -> Result<Option<PassthroughAuthorizationCode>, RedeemError> {
    let Some(text) = code.filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    let Some(sealed) = open_passthrough_code(text, cipher) else {
        return Ok(None);
    };
    if sealed.mcp_server_id != server_id {
        return Err(RedeemError::WrongServer);
    }
    let Some(verifier) = code_verifier else {
        return Err(RedeemError::VerifierRequired);
    };
    if verifier.is_empty() {
        return Err(RedeemError::VerifierRequired);
    }
    Ok(Some(sealed))
}
