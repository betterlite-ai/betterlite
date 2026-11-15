pub mod cache;
pub mod post;
mod signing;
mod tests;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use betterlite_mcp::credential_contracts::auth_config::id_jag::ClientAuth;
use betterlite_mcp::credential_contracts::errors::{CredError, CredResult};
use secrecy::SecretString;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::error::Category;

use self::post::{PostedForm, TokenEndpointPost};
use crate::mcp::outbound_credentials::{SystemWallClock, WallClock};

pub const CLIENT_ASSERTION_TYPE: &str = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";
pub const ASSERTION_LIFETIME_SECONDS: i64 = 60;

const NON_JSON_DETAIL: &str = "token exchange failed: token endpoint returned a non-JSON response";
const NO_ACCESS_TOKEN: &str = "token exchange failed: token endpoint response missing access_token";
const INVALID_EXPIRY_DETAIL: &str =
    "token exchange failed: token endpoint response has an invalid expires_in";

#[derive(Clone, Eq, PartialEq)]
pub struct ExchangedToken {
    pub access_token: String,
    pub expires_in: Option<i64>,
}

impl fmt::Debug for ExchangedToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExchangedToken")
            .field("access_token", &"***")
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

pub struct TokenEndpointClient<Post> {
    pub post: Post,
    pub clock: Arc<dyn WallClock>,
    pub new_jti: fn() -> String,
}

impl<Post> TokenEndpointClient<Post> {
    #[must_use]
    pub fn new(post: Post) -> Self {
        Self {
            post,
            clock: Arc::new(SystemWallClock),
            new_jti: || uuid::Uuid::new_v4().simple().to_string(),
        }
    }
}

impl<Post: TokenEndpointPost> TokenEndpointClient<Post> {
    pub async fn fetch(
        &self,
        endpoint: &str,
        client_id: &str,
        grant_params: &BTreeMap<String, String>,
        client_auth: &ClientAuth,
    ) -> CredResult<ExchangedToken> {
        let form = self.form(endpoint, client_id, grant_params, client_auth)?;
        match self.post.post(endpoint, &form).await {
            PostedForm::Answered { status, body } => parse_response(status, &body),
            PostedForm::Unreachable { kind } => Err(CredError::UpstreamUnavailable(format!(
                "token exchange failed: token endpoint unreachable ({kind})"
            ))),
        }
    }

    fn form(
        &self,
        endpoint: &str,
        client_id: &str,
        grant_params: &BTreeMap<String, String>,
        client_auth: &ClientAuth,
    ) -> CredResult<Vec<(String, SecretString)>> {
        let mut form: BTreeMap<String, SecretString> = grant_params
            .iter()
            .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
            .collect();
        // The client's own parameters win over a grant that names the same key, as LiteLLM's dict merge does.
        form.insert(
            "client_id".to_owned(),
            SecretString::from(client_id.to_owned()),
        );
        match client_auth {
            ClientAuth::PrivateKeyJwt(auth) => {
                form.insert(
                    "client_assertion_type".to_owned(),
                    SecretString::from(CLIENT_ASSERTION_TYPE),
                );
                let assertion = signing::client_assertion(
                    auth,
                    endpoint,
                    client_id,
                    self.clock.now().timestamp(),
                    &(self.new_jti)(),
                )?;
                form.insert("client_assertion".to_owned(), SecretString::from(assertion));
            }
            ClientAuth::ClientSecret(auth) => {
                form.insert("client_secret".to_owned(), auth.client_secret.clone());
            }
        }
        Ok(form.into_iter().collect())
    }
}

#[derive(Deserialize)]
struct TokenEndpointResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    expires_in: Option<ExpiresInField>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ExpiresInField {
    Number(serde_json::Number),
    Other(IgnoredAny),
}

// httpx raises for 4xx and 5xx only, so a 3xx that survived redirect following is parsed like a 2xx.
fn parse_response(status: u16, body: &[u8]) -> CredResult<ExchangedToken> {
    if status >= 400 {
        return Err(CredError::UpstreamUnavailable(format!(
            "token exchange failed with status {status}"
        )));
    }
    let object = body.trim_ascii_start().starts_with(b"{");
    let response = match serde_json::from_slice::<TokenEndpointResponse>(body) {
        Ok(response) if object => response,
        Err(error) if matches!(error.classify(), Category::Syntax | Category::Eof) => {
            return Err(CredError::UpstreamUnavailable(NON_JSON_DETAIL.to_owned()));
        }
        Ok(_) | Err(_) => {
            return Err(CredError::UpstreamUnavailable(NO_ACCESS_TOKEN.to_owned()));
        }
    };
    let Some(access_token) = response.access_token else {
        return Err(CredError::UpstreamUnavailable(NO_ACCESS_TOKEN.to_owned()));
    };
    let expires_in = match response.expires_in {
        None => None,
        Some(ExpiresInField::Number(number)) => Some(
            number
                .as_i64()
                .ok_or_else(|| CredError::UpstreamUnavailable(INVALID_EXPIRY_DETAIL.to_owned()))?,
        ),
        Some(ExpiresInField::Other(_)) => {
            return Err(CredError::UpstreamUnavailable(
                INVALID_EXPIRY_DETAIL.to_owned(),
            ));
        }
    };
    Ok(ExchangedToken {
        access_token,
        expires_in,
    })
}
