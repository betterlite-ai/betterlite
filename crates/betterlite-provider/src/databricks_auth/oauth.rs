use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::databricks_auth::error::DatabricksError;
use crate::databricks_auth::url_parts;

pub const OAUTH_TIMEOUT: Duration = Duration::from_secs(30);
pub const TOKEN_PATH: &str = "/oidc/v1/token";
pub const TOKEN_BODY: &str = "grant_type=client_credentials&scope=all-apis";
pub const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";

#[derive(Clone, PartialEq, Eq)]
pub struct OauthTokenRequest {
    pub url: String,
    pub body: String,
    pub authorization: String,
    pub content_type: String,
    pub timeout: Duration,
}

// the authorization header holds base64 of client_id:client_secret, so Debug must not print it
impl fmt::Debug for OauthTokenRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OauthTokenRequest")
            .field("url", &self.url)
            .field("body", &self.body)
            .field("authorization", &"[redacted]")
            .field("content_type", &self.content_type)
            .field("timeout", &self.timeout)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub enum OauthReply {
    Reply { status: u16, body: SecretString },
    Transport(String),
}

#[async_trait]
pub trait OauthTokenSource: Send + Sync {
    async fn send(&self, request: &OauthTokenRequest) -> OauthReply;
}

#[derive(Deserialize)]
struct TokenReply {
    access_token: Option<String>,
}

#[must_use]
pub fn oauth_token_request(
    api_base: &str,
    client_id: &str,
    client_secret: &str,
) -> OauthTokenRequest {
    let workspace = url_parts::workspace(api_base);
    let credentials = STANDARD.encode(format!("{client_id}:{client_secret}"));
    OauthTokenRequest {
        url: format!("{workspace}{TOKEN_PATH}"),
        body: TOKEN_BODY.to_owned(),
        authorization: format!("Basic {credentials}"),
        content_type: FORM_CONTENT_TYPE.to_owned(),
        timeout: OAUTH_TIMEOUT,
    }
}

pub fn parse_oauth_token_reply(status: u16, body: &str) -> Result<SecretString, DatabricksError> {
    if status != 200 {
        return Err(DatabricksError::new(
            status,
            format!("OAuth M2M token request failed: {body}"),
        ));
    }
    // LiteLLM reads response.json()["access_token"], so a reply without the field is a typed 500 here
    let reply: TokenReply = serde_json::from_str(body).map_err(|source| {
        DatabricksError::new(
            500,
            format!("the OAuth M2M reply has no access_token: {source}"),
        )
    })?;
    reply.access_token.map(SecretString::from).ok_or_else(|| {
        DatabricksError::new(500, "the OAuth M2M reply has no access_token".to_owned())
    })
}

pub async fn request_oauth_token(
    request: &OauthTokenRequest,
    tokens: &dyn OauthTokenSource,
) -> Result<SecretString, DatabricksError> {
    match tokens.send(request).await {
        OauthReply::Reply { status, body } => parse_oauth_token_reply(status, body.expose_secret()),
        OauthReply::Transport(error) => Err(DatabricksError::new(
            500,
            format!("OAuth M2M token request failed: {error}"),
        )),
    }
}
