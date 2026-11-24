mod tests;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::gateway_dcr::contracts::{GATEWAY_DCR_CLIENT_ID_PREFIX, MAX_REDIRECT_URIS};
use crate::types::SessionAudience;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ModelError {
    #[error("the sealed DCR client must carry 1 to 4 redirect URIs")]
    RedirectUriCount,
    #[error("a required field of the sealed value is empty")]
    EmptyField,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, try_from = "GatewayDcrClientPayload")]
pub struct GatewayDcrClient {
    pub redirect_uris: Vec<String>,
    pub iat: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GatewayDcrClientPayload {
    redirect_uris: Vec<String>,
    iat: i64,
}

impl TryFrom<GatewayDcrClientPayload> for GatewayDcrClient {
    type Error = ModelError;

    fn try_from(payload: GatewayDcrClientPayload) -> Result<Self, Self::Error> {
        if payload.redirect_uris.is_empty() || payload.redirect_uris.len() > MAX_REDIRECT_URIS {
            return Err(ModelError::RedirectUriCount);
        }
        Ok(Self {
            redirect_uris: payload.redirect_uris,
            iat: payload.iat,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, try_from = "ConnectFlowPayload")]
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectFlowPayload {
    user_id: String,
    client_id: String,
    redirect_uri: String,
    state: String,
    code_challenge: String,
    jti: String,
    exp: i64,
    resource_server_id: Option<String>,
    audience: Option<SessionAudience>,
}

impl TryFrom<ConnectFlowPayload> for ConnectFlow {
    type Error = ModelError;

    fn try_from(payload: ConnectFlowPayload) -> Result<Self, Self::Error> {
        if payload.user_id.is_empty()
            || payload.client_id.is_empty()
            || payload.redirect_uri.is_empty()
            || payload.code_challenge.is_empty()
            || payload.jti.is_empty()
        {
            return Err(ModelError::EmptyField);
        }
        Ok(Self {
            user_id: payload.user_id,
            client_id: payload.client_id,
            redirect_uri: payload.redirect_uri,
            state: payload.state,
            code_challenge: payload.code_challenge,
            jti: payload.jti,
            exp: payload.exp,
            resource_server_id: payload.resource_server_id,
            audience: payload.audience,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, try_from = "GatewayAuthCodePayload")]
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GatewayAuthCodePayload {
    user_id: String,
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    jti: String,
    iat: i64,
    exp: i64,
    resource_server_id: Option<String>,
    audience: Option<SessionAudience>,
    team_id: Option<String>,
}

impl TryFrom<GatewayAuthCodePayload> for GatewayAuthCode {
    type Error = ModelError;

    fn try_from(payload: GatewayAuthCodePayload) -> Result<Self, Self::Error> {
        if payload.user_id.is_empty()
            || payload.client_id.is_empty()
            || payload.redirect_uri.is_empty()
            || payload.code_challenge.is_empty()
            || payload.jti.is_empty()
        {
            return Err(ModelError::EmptyField);
        }
        Ok(Self {
            user_id: payload.user_id,
            client_id: payload.client_id,
            redirect_uri: payload.redirect_uri,
            code_challenge: payload.code_challenge,
            jti: payload.jti,
            iat: payload.iat,
            exp: payload.exp,
            resource_server_id: payload.resource_server_id,
            audience: payload.audience,
            team_id: payload.team_id,
        })
    }
}

#[must_use]
pub fn is_gateway_dcr_client_id(client_id: Option<&str>) -> bool {
    let Some(client_id) = client_id else {
        return false;
    };
    client_id.starts_with(GATEWAY_DCR_CLIENT_ID_PREFIX)
}
