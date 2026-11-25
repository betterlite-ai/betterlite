mod tests;

use serde::{Deserialize, Serialize};

use crate::gateway_dcr::contracts::{GATEWAY_DCR_CLIENT_ID_PREFIX, MAX_REDIRECT_URIS};
use crate::gateway_dcr::sealed::Sealable;
use crate::types::SessionAudience;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayDcrClient {
    pub redirect_uris: Vec<String>,
    pub iat: i64,
}

impl Sealable for GatewayDcrClient {
    fn acceptable(&self) -> bool {
        !self.redirect_uris.is_empty() && self.redirect_uris.len() <= MAX_REDIRECT_URIS
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectFlow {
    pub user_id: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub state: String,
    pub code_challenge: String,
    pub jti: String,
    pub exp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_server_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<SessionAudience>,
}

impl Sealable for ConnectFlow {
    fn acceptable(&self) -> bool {
        !self.user_id.is_empty()
            && !self.client_id.is_empty()
            && !self.redirect_uri.is_empty()
            && !self.code_challenge.is_empty()
            && !self.jti.is_empty()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayAuthCode {
    pub user_id: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub jti: String,
    pub iat: i64,
    pub exp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_server_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<SessionAudience>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
}

impl Sealable for GatewayAuthCode {
    fn acceptable(&self) -> bool {
        !self.user_id.is_empty()
            && !self.client_id.is_empty()
            && !self.redirect_uri.is_empty()
            && !self.code_challenge.is_empty()
            && !self.jti.is_empty()
    }
}
#[must_use]
pub fn is_gateway_dcr_client_id(client_id: Option<&str>) -> bool {
    let Some(id) = client_id else {
        return false;
    };
    id.starts_with(GATEWAY_DCR_CLIENT_ID_PREFIX)
}
