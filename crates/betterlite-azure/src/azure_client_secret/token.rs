use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::IgnoredAny;

use super::ClientSecretError;
use super::ResolvedCredentials;
use super::transport::{Clock, HttpMethod, HttpRequest, Transport};

#[derive(Debug, Clone)]
pub struct AccessToken {
    pub token: SecretString,
    pub expires_on: i64,
    pub token_type: String,
    pub refresh_on: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct TokenResponse {
    pub access_token: SecretString,
    pub expires_in: i64,
    pub token_type: String,
    pub refresh_on: Option<i64>,
    pub refresh_in: Option<i64>,
}

#[derive(Deserialize)]
struct RawTokenResponse {
    access_token: Option<TextField>,
    expires_in: Option<NumberField>,
    token_type: Option<TextField>,
    refresh_on: Option<NumberField>,
    refresh_in: Option<NumberField>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum TextField {
    Text(String),
    Other(IgnoredAny),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum NumberField {
    Number(i64),
    Text(String),
    Other(IgnoredAny),
}

pub fn decode_client_secret_token(bytes: &[u8]) -> Result<TokenResponse, ClientSecretError> {
    let parsed: RawTokenResponse =
        serde_json::from_slice(bytes).map_err(|source| ClientSecretError::TokenJson { source })?;
    let access_token = match parsed.access_token {
        Some(TextField::Text(token)) => SecretString::from(token),
        Some(TextField::Other(_)) => {
            return Err(ClientSecretError::TokenFieldType {
                field: "access_token",
            });
        }
        None => return Err(ClientSecretError::TokenAccessTokenMissing),
    };
    let Some(expires_in_field) = parsed.expires_in else {
        return Err(ClientSecretError::TokenExpiresInMissing);
    };
    let expires_in = integer(expires_in_field, "expires_in")?;
    let token_type = match parsed.token_type {
        Some(TextField::Text(kind)) => kind,
        Some(TextField::Other(_)) => {
            return Err(ClientSecretError::TokenFieldType {
                field: "token_type",
            });
        }
        None => "Bearer".to_owned(),
    };
    let absolute_refresh = parsed
        .refresh_on
        .map(|value| integer(value, "refresh_on"))
        .transpose()?;
    let relative_refresh = parsed
        .refresh_in
        .map(|value| integer(value, "refresh_in"))
        .transpose()?;
    Ok(TokenResponse {
        access_token,
        expires_in,
        token_type,
        refresh_on: absolute_refresh,
        refresh_in: relative_refresh,
    })
}

fn integer(value: NumberField, field: &'static str) -> Result<i64, ClientSecretError> {
    match value {
        NumberField::Number(number) => Ok(number),
        NumberField::Text(text) => text
            .parse::<i64>()
            .map_err(|_error| ClientSecretError::TokenInteger { field }),
        NumberField::Other(_) => Err(ClientSecretError::TokenFieldType { field }),
    }
}

#[must_use]
pub fn token_request(token_endpoint: &str, credentials: &ResolvedCredentials) -> HttpRequest {
    HttpRequest {
        method: HttpMethod::Post,
        url: token_endpoint.to_owned(),
        query: Vec::new(),
        headers: vec![("Accept".to_owned(), "application/json".to_owned())],
        form: vec![
            ("client_id".to_owned(), credentials.client_id.clone()),
            ("grant_type".to_owned(), "client_credentials".to_owned()),
            ("client_info".to_owned(), "1".to_owned()),
            (
                "client_secret".to_owned(),
                credentials.client_secret.expose_secret().to_owned(),
            ),
            ("scope".to_owned(), credentials.scope.clone()),
        ],
    }
}

pub fn request_token(
    transport: &dyn Transport,
    clock: &dyn Clock,
    token_endpoint: &str,
    credentials: &ResolvedCredentials,
) -> Result<AccessToken, ClientSecretError> {
    let request = token_request(token_endpoint, credentials);
    let reply = transport
        .send(&request)
        .map_err(|source| ClientSecretError::TokenTransport { source })?;
    if reply.status >= 500 {
        return Err(ClientSecretError::Authentication {
            message: format!("HTTP Error: {}", reply.status),
        });
    }
    let response = decode_client_secret_token(reply.body.expose_secret().as_bytes())?;
    let now = clock.now_unix_seconds();
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
