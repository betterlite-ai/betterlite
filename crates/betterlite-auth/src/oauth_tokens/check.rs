use std::collections::BTreeMap;

use betterlite_config::sections::oauth_tokens::Settings;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::controls::json_value::JsonValue;
use http::Method;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;
use thiserror::Error;
use url::Url;

use crate::oauth_tokens::extract::{ExtractError, ExtractedIdentity, extract_user_info};
use crate::oauth_tokens::request::{
    EndpointMode, PrepareError, endpoint_mode, prepare_introspection_request,
    prepare_token_info_request,
};
use crate::oauth_tokens::transport::{
    TokenInfoRequest, TokenInfoResponse, TokenInfoTransport, TokenInfoTransportError,
};
use crate::oauth_tokens::{DecodeError, TypedValue, decode_oauth_response};
use crate::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};

const STATUS_PREFIX: &str = "Oauth 2.0 Token validation failed";
const VALIDATION_PREFIX: &str = "An error occurred during token validation";

#[derive(Debug, Error)]
pub enum CheckError {
    #[error("OAUTH_TOKEN_INFO_ENDPOINT environment variable is not set")]
    EndpointAbsent,
    // check_oauth2_token wraps httpx's HTTPStatusError in this message category
    #[error("{STATUS_PREFIX}: HTTP status {status}")]
    Status { status: u16 },
    // every other failure, the inactive-token ValueError included, takes the generic category
    #[error("{VALIDATION_PREFIX}: {source}")]
    Validation { source: ValidationError },
}

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error(transparent)]
    Transport(#[from] TokenInfoTransportError),
    #[error(transparent)]
    Prepare(#[from] PrepareError),
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error(transparent)]
    Extract(#[from] ExtractError),
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error("the OAuth token info endpoint is not a URL: {source}")]
    Url {
        #[source]
        source: url::ParseError,
    },
    #[error("Token is not active")]
    Inactive,
}

impl From<ValidationError> for CheckError {
    fn from(source: ValidationError) -> Self {
        Self::Validation { source }
    }
}

// check_oauth2_token: an absent or empty endpoint fails before any request leaves the process
pub async fn check_oauth2_token(
    settings: &Settings,
    transport: &dyn TokenInfoTransport,
    token: &SecretString,
) -> Result<UserAPIKeyAuth, CheckError> {
    let Some(endpoint) = settings
        .token_info_endpoint
        .as_deref()
        .filter(|value| !value.is_empty())
    else {
        return Err(CheckError::EndpointAbsent);
    };
    let mode = endpoint_mode(settings);
    let request = token_request(settings, token, endpoint, mode)?;
    match mode {
        EndpointMode::Introspection => {
            tracing::debug!("Using OAuth2 introspection endpoint (POST)");
        }
        EndpointMode::TokenInfo => {
            tracing::debug!("Using generic token info endpoint (GET)");
        }
    }
    let response = transport
        .send(request)
        .await
        .map_err(ValidationError::from)?;
    if !(200..=299).contains(&response.status) {
        // httpx's raise_for_status reports the status line; the response body stays unread
        return Err(CheckError::Status {
            status: response.status,
        });
    }
    // LiteLLM logs the token and the whole response here; betterlite logs neither
    let identity = decode_identity(settings, &response, mode)?;
    Ok(identity_auth(token, identity)?)
}

fn token_request(
    settings: &Settings,
    token: &SecretString,
    endpoint: &str,
    mode: EndpointMode,
) -> Result<TokenInfoRequest, ValidationError> {
    let url = Url::parse(endpoint).map_err(|source| ValidationError::Url { source })?;
    match mode {
        EndpointMode::Introspection => {
            let prepared = prepare_introspection_request(
                token,
                settings.client_id.as_deref(),
                settings.client_secret.as_ref(),
            )?;
            Ok(TokenInfoRequest {
                method: Method::POST,
                url,
                headers: prepared.headers,
                form: prepared.form,
            })
        }
        EndpointMode::TokenInfo => {
            let prepared = prepare_token_info_request(token)?;
            Ok(TokenInfoRequest {
                method: Method::GET,
                url,
                headers: prepared.headers,
                form: Vec::new(),
            })
        }
    }
}

fn decode_identity(
    settings: &Settings,
    response: &TokenInfoResponse,
    mode: EndpointMode,
) -> Result<ExtractedIdentity, ValidationError> {
    let document = decode_oauth_response(&response.body)?;
    // data.get("active", True): an absent key passes, a present one is read for truth
    if mode == EndpointMode::Introspection {
        let JsonValue::Object(fields) = &document else {
            return Err(ValidationError::from(ExtractError::NotAnObject));
        };
        if !fields.get("active").is_none_or(json_truthy) {
            return Err(ValidationError::Inactive);
        }
    }
    let identity = extract_user_info(
        &document,
        &settings.user_id_field_name,
        &settings.user_role_field_name,
        &settings.user_team_id_field_name,
    )?;
    Ok(identity)
}

// Python truthiness over the decoded document: null, false, zero, "" and empty containers are false
fn json_truthy(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(flag) => *flag,
        JsonValue::Number(number) => number_truthy(number),
        JsonValue::Text(text) => !text.is_empty(),
        JsonValue::Array(items) => !items.is_empty(),
        JsonValue::Object(fields) => !fields.is_empty(),
    }
}

fn number_truthy(number: &Number) -> bool {
    if let Some(integer) = number.as_i64() {
        return integer != 0;
    }
    if let Some(unsigned) = number.as_u64() {
        return unsigned != 0;
    }
    number.as_f64().is_none_or(|value| value.abs() > 0.0)
}

// UserAPIKeyAuth(api_key=token, team_id=..., user_id=..., user_role=...): the B2314 boundary validates the role
fn identity_auth(
    token: &SecretString,
    identity: ExtractedIdentity,
) -> Result<UserAPIKeyAuth, ValidationError> {
    let mut fields = BTreeMap::new();
    fields.insert(
        "api_key".to_owned(),
        JsonData::String(token.expose_secret().to_owned()),
    );
    if let Some(user_id) = identity.user_id {
        fields.insert("user_id".to_owned(), JsonData::String(user_id));
    }
    if let Some(user_role) = identity.user_role {
        fields.insert("user_role".to_owned(), JsonData::String(user_role));
    }
    if let Some(team_id) = identity.team_id {
        fields.insert("team_id".to_owned(), JsonData::String(team_id));
    }
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields))?;
    Ok(auth)
}
