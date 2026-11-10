use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::IgnoredAny;

use super::ASSERTION_TYPE;
use crate::azure_client_secret::ClientSecretError;
use crate::azure_client_secret::token::{AccessToken, decode_client_secret_token};
use crate::azure_client_secret::transport::{HttpMethod, HttpReply, HttpRequest};

#[derive(Deserialize)]
struct Outcome {
    access_token: Option<IgnoredAny>,
    error_description: Option<String>,
    error: Option<String>,
}

// the form has a client_assertion where the client secret grant has a client_secret
#[must_use]
pub fn assertion_request(
    token_endpoint: &str,
    client_id: &str,
    scope: &str,
    assertion: &SecretString,
) -> HttpRequest {
    HttpRequest {
        method: HttpMethod::Post,
        url: token_endpoint.to_owned(),
        query: Vec::new(),
        headers: vec![("Accept".to_owned(), "application/json".to_owned())],
        form: vec![
            ("client_id".to_owned(), client_id.to_owned()),
            ("grant_type".to_owned(), "client_credentials".to_owned()),
            (
                "client_assertion".to_owned(),
                assertion.expose_secret().to_owned(),
            ),
            ("client_info".to_owned(), "1".to_owned()),
            (
                "client_assertion_type".to_owned(),
                ASSERTION_TYPE.to_owned(),
            ),
            ("scope".to_owned(), scope.to_owned()),
        ],
    }
}

// azure-identity raises the IdP's error_description, else its error, else the text None
pub fn read_token_reply(reply: &HttpReply, now: i64) -> Result<AccessToken, ClientSecretError> {
    if reply.status >= 500 {
        return Err(ClientSecretError::Authentication {
            message: format!("HTTP Error: {}", reply.status),
        });
    }
    let body = reply.body.expose_secret().as_bytes();
    let outcome: Outcome =
        serde_json::from_slice(body).map_err(|source| ClientSecretError::TokenJson { source })?;
    if outcome.access_token.is_none() {
        let message = outcome
            .error_description
            .filter(|text| !text.is_empty())
            .or(outcome.error)
            .unwrap_or_else(|| "None".to_owned());
        return Err(ClientSecretError::Authentication { message });
    }
    let response = decode_client_secret_token(body)?;
    let expires_on = now
        .checked_add(response.expires_in)
        .ok_or(ClientSecretError::TokenExpiryOverflow)?;
    // MSAL turns a refresh_in lifetime into an absolute refresh_on and drops the field
    let refresh_on = match response.refresh_in {
        Some(refresh_in) => Some(
            now.checked_add(refresh_in)
                .ok_or(ClientSecretError::TokenExpiryOverflow)?,
        ),
        None => response.refresh_on,
    };
    Ok(AccessToken {
        token: response.access_token,
        expires_on,
        token_type: response.token_type,
        refresh_on,
    })
}
