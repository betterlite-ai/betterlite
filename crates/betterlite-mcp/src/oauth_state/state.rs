mod tests;

use betterlite_db::mcp_stored_contracts::MCPTokenEndpointAuthMethod;
use betterlite_db::mcp_stored_contracts::secret::StoredSecret;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::SealCipher;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OauthState {
    pub base_url: String,
    pub client_redirect_uri: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub dcr_client_id: Option<String>,
    pub dcr_client_secret: Option<StoredSecret>,
    pub dcr_token_endpoint_auth_method: Option<MCPTokenEndpointAuthMethod>,
    pub litellm_user_id: Option<String>,
    pub mcp_server_id: Option<String>,
    pub oauth_nonce: Option<String>,
    pub original_state: String,
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum StateError {
    #[error("Failed to decrypt state parameter")]
    Undecryptable,
    #[error("the state parameter does not hold a JSON object")]
    Malformed,
}

#[must_use]
pub fn encode_state(cipher: &dyn SealCipher, state: &OauthState) -> String {
    match serde_json::to_string(state) {
        Ok(json) => cipher.encrypt(&json),
        Err(_error) => String::new(),
    }
}

pub fn decode_state(
    cipher: &dyn SealCipher,
    encoded_state: &str,
) -> Result<OauthState, StateError> {
    cipher.decrypt(encoded_state, "oauth_state").map_or_else(
        || Err(StateError::Undecryptable),
        |json| serde_json::from_str(&json).map_err(|_error| StateError::Malformed),
    )
}
