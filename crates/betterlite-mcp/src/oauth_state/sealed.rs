mod tests;

use betterlite_db::mcp_stored_contracts::MCPTokenEndpointAuthMethod;
use betterlite_db::mcp_stored_contracts::secret::StoredSecret;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::SealCipher;

pub const BRIDGE_CODE_PREFIX: &str = "llm_bcode_";
pub const PASSTHROUGH_CODE_PREFIX: &str = "llm_ptcode_";

const BRIDGE_CODE_LABEL: &str = "bridge_authorization_code";
const PASSTHROUGH_CODE_LABEL: &str = "passthrough_authorization_code";

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

// five values plus the cipher would be six arguments, past the workspace's five-argument limit
#[derive(Clone, Copy)]
pub struct PassthroughCodeParts<'code> {
    pub client_id: &'code str,
    pub client_secret: Option<&'code str>,
    pub mcp_server_id: &'code str,
    pub token_endpoint_auth_method: Option<MCPTokenEndpointAuthMethod>,
    pub upstream_code: &'code str,
}

impl PassthroughAuthorizationCode {
    fn from_parts(parts: &PassthroughCodeParts<'_>) -> Self {
        Self {
            client_id: parts.client_id.to_owned(),
            client_secret: parts
                .client_secret
                .map(|secret| StoredSecret(SecretString::from(secret.to_owned()))),
            mcp_server_id: parts.mcp_server_id.to_owned(),
            token_endpoint_auth_method: parts.token_endpoint_auth_method,
            upstream_code: parts.upstream_code.to_owned(),
        }
    }
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

trait RequiredFields {
    fn has_all_required_fields(&self) -> bool;
}

impl RequiredFields for BridgeAuthorizationCode {
    fn has_all_required_fields(&self) -> bool {
        !self.litellm_user_id.is_empty()
            && !self.mcp_server_id.is_empty()
            && !self.upstream_code.is_empty()
    }
}

impl RequiredFields for PassthroughAuthorizationCode {
    fn has_all_required_fields(&self) -> bool {
        !self.client_id.is_empty()
            && !self.mcp_server_id.is_empty()
            && !self.upstream_code.is_empty()
    }
}

fn seal_payload<TPayload>(prefix: &str, payload: &TPayload, cipher: &dyn SealCipher) -> String
where
    TPayload: Serialize,
{
    let json = serde_json::to_string(payload).unwrap_or_default();
    format!("{prefix}{}", cipher.encrypt(&json))
}

fn open_payload<TPayload>(
    prefix: &str,
    label: &str,
    code: &str,
    cipher: &dyn SealCipher,
) -> Option<TPayload>
where
    TPayload: DeserializeOwned,
{
    let sealed = code.strip_prefix(prefix)?;
    let json = cipher.decrypt(sealed, label)?;
    serde_json::from_str(&json).ok()
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
    seal_payload(BRIDGE_CODE_PREFIX, &sealed, cipher)
}

#[must_use]
pub fn open_bridge_code(code: &str, cipher: &dyn SealCipher) -> Option<BridgeAuthorizationCode> {
    open_payload(BRIDGE_CODE_PREFIX, BRIDGE_CODE_LABEL, code, cipher)
        .filter(RequiredFields::has_all_required_fields)
}

#[must_use]
pub fn seal_passthrough_code(parts: &PassthroughCodeParts<'_>, cipher: &dyn SealCipher) -> String {
    seal_payload(
        PASSTHROUGH_CODE_PREFIX,
        &PassthroughAuthorizationCode::from_parts(parts),
        cipher,
    )
}

#[must_use]
pub fn open_passthrough_code(
    code: &str,
    cipher: &dyn SealCipher,
) -> Option<PassthroughAuthorizationCode> {
    open_payload(
        PASSTHROUGH_CODE_PREFIX,
        PASSTHROUGH_CODE_LABEL,
        code,
        cipher,
    )
    .filter(RequiredFields::has_all_required_fields)
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
