use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::IgnoredAny;
use thiserror::Error;

use crate::azure_client_secret::ClientSecretError;
use crate::azure_client_secret::authority::Authority;
use crate::azure_client_secret::token::decode_client_secret_token;
use crate::azure_client_secret::transport::HttpMethod;

use super::record::{self, AuthenticationRecord, RecordError};
use super::transport::{HttpRequest, Transport};
use super::{DEFAULT_SCOPE, UsernamePasswordError, UsernamePasswordInput, scope};

#[derive(Debug, Clone, Deserialize)]
pub struct IdentityClaims {
    pub aud: Option<String>,
    pub iss: Option<String>,
    pub sub: Option<String>,
    pub tid: Option<String>,
    pub preferred_username: Option<String>,
    pub upn: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PasswordToken {
    pub access_token: SecretString,
    pub expires_in: i64,
    pub token_type: String,
    pub refresh_on: Option<i64>,
    pub refresh_in: Option<i64>,
    pub record: AuthenticationRecord,
}

#[derive(Debug, Error)]
pub enum PasswordTokenError {
    #[error("the token response is not JSON")]
    Json(#[source] serde_json::Error),
    #[error("the token response has the wrong shape")]
    Shape(#[source] serde_json::Error),
    #[error("the token response is missing access_token")]
    AccessTokenMissing {
        error: Option<String>,
        error_description: Option<String>,
    },
    #[error("the token response field {field} has the wrong type")]
    FieldType { field: &'static str },
    #[error("Failed to build AuthenticationRecord from unexpected identity token")]
    Record(#[source] RecordError),
    #[error(transparent)]
    Shared(#[from] ClientSecretError),
}

#[derive(Deserialize)]
struct AccessEnvelope {
    access_token: Option<TextField>,
    error: Option<TextField>,
    error_description: Option<TextField>,
}

#[derive(Deserialize)]
struct ClaimsEnvelope {
    id_token_claims: Option<IdentityClaims>,
    client_info: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum TextField {
    Text(String),
    Other(IgnoredAny),
}

impl TextField {
    fn into_text(self) -> Option<String> {
        match self {
            Self::Text(text) => Some(text),
            Self::Other(_ignored) => None,
        }
    }
}

#[derive(Deserialize)]
struct RealmEnvelope {
    account_type: Option<String>,
}

pub fn decode_password_token(bytes: &[u8]) -> Result<PasswordToken, PasswordTokenError> {
    let envelope: AccessEnvelope =
        serde_json::from_slice(bytes).map_err(PasswordTokenError::Json)?;
    let access_token = match envelope.access_token {
        Some(TextField::Text(token)) => SecretString::from(token),
        Some(TextField::Other(_ignored)) => {
            return Err(PasswordTokenError::FieldType {
                field: "access_token",
            });
        }
        None => {
            return Err(PasswordTokenError::AccessTokenMissing {
                error: envelope.error.and_then(TextField::into_text),
                error_description: envelope.error_description.and_then(TextField::into_text),
            });
        }
    };
    let claims: ClaimsEnvelope =
        serde_json::from_slice(bytes).map_err(PasswordTokenError::Shape)?;
    // the SDK builds the authentication record before it reads the expiry, so a bare access token fails here
    let record = record::build(
        claims.id_token_claims.as_ref(),
        claims.client_info.as_deref(),
    )
    .map_err(PasswordTokenError::Record)?;
    let token = decode_client_secret_token(bytes).map_err(PasswordTokenError::Shared)?;
    Ok(PasswordToken {
        access_token,
        expires_in: token.expires_in,
        token_type: token.token_type,
        refresh_on: token.refresh_on,
        refresh_in: token.refresh_in,
        record,
    })
}

// MSAL drops a falsy form value, so an empty client id, username or password leaves its field out
fn password_form(input: &UsernamePasswordInput, scope: &str) -> Vec<(String, String)> {
    [
        ("client_id", input.client_id.as_str()),
        ("grant_type", "password"),
        ("client_info", "1"),
        ("username", input.username.as_str()),
        ("password", input.password.expose_secret()),
        ("scope", scope),
    ]
    .into_iter()
    .filter(|(_name, value)| !value.is_empty())
    .map(|(name, value)| (name.to_owned(), value.to_owned()))
    .collect()
}

// MSAL asks the user realm endpoint before every password grant; a 404 means the host has none, so the account is managed
fn reject_federated(
    transport: &dyn Transport,
    instance: &str,
    username: &str,
) -> Result<(), UsernamePasswordError> {
    let request = HttpRequest {
        method: HttpMethod::Get,
        url: format!("https://{instance}/common/userrealm/{username}?api-version=1.0"),
        query: Vec::new(),
        headers: vec![("Accept".to_owned(), "application/json".to_owned())],
        form: Vec::new(),
    };
    let reply = transport
        .send(&request)
        .map_err(|source| UsernamePasswordError::Transport { source })?;
    if reply.status == 404 {
        return Ok(());
    }
    if reply.status >= 400 {
        return Err(UsernamePasswordError::Authentication {
            message: format!("HTTP Error: {}", reply.status),
        });
    }
    let realm: RealmEnvelope = serde_json::from_slice(reply.body.expose_secret().as_bytes())
        .map_err(UsernamePasswordError::Realm)?;
    if realm.account_type.as_deref() == Some("Federated") {
        return Err(UsernamePasswordError::FederatedAccount);
    }
    Ok(())
}

pub fn request_token(
    transport: &dyn Transport,
    authority: &Authority,
    input: &UsernamePasswordInput,
) -> Result<PasswordToken, UsernamePasswordError> {
    let decorated = scope::decorate(input.scope.as_deref().unwrap_or(DEFAULT_SCOPE))?;
    reject_federated(transport, &authority.instance, &input.username)?;
    let request = HttpRequest {
        method: HttpMethod::Post,
        url: authority.token_endpoint.clone(),
        query: Vec::new(),
        headers: vec![("Accept".to_owned(), "application/json".to_owned())],
        form: password_form(input, &decorated),
    };
    let reply = transport
        .send(&request)
        .map_err(|source| UsernamePasswordError::Transport { source })?;
    if reply.status >= 500 {
        return Err(UsernamePasswordError::Authentication {
            message: format!("HTTP Error: {}", reply.status),
        });
    }
    match decode_password_token(reply.body.expose_secret().as_bytes()) {
        Ok(token) => Ok(token),
        Err(PasswordTokenError::AccessTokenMissing {
            error,
            error_description,
        }) => Err(UsernamePasswordError::Authentication {
            message: error_description
                .filter(|text| !text.is_empty())
                .or(error)
                .unwrap_or_default(),
        }),
        Err(source) => Err(UsernamePasswordError::Token(source)),
    }
}
