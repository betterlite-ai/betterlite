use std::time::SystemTime;

use betterlite_models::decisions::{Extra, Json};
use betterlite_sigv4::credentials::Credentials;
use betterlite_sigv4::error::SigV4Error;
use betterlite_sigv4::sign::{Signable, SigningMode, sign_request};
use secrecy::SecretString;

use crate::openai_base_config::Headers;

pub const SERVICE: &str = "bedrock-agentcore";
const ACCESS_KEY_ID: &str = "aws_access_key_id";
const SECRET_ACCESS_KEY: &str = "aws_secret_access_key";
const SESSION_TOKEN: &str = "aws_session_token";
const ROLE_NAME: &str = "aws_role_name";
const PROFILE_NAME: &str = "aws_profile_name";
const SESSION_NAME: &str = "aws_session_name";
const WEB_IDENTITY_TOKEN: &str = "aws_web_identity_token";
const STS_ENDPOINT: &str = "aws_sts_endpoint";
const EXTERNAL_ID: &str = "aws_external_id";
const SESSION_TAGS: &str = "aws_session_tags";
// the STS assume-role, profile and web-identity flows live in betterlite-bedrock's resolve_credentials
const UNRESOLVED_AUTH_PARAMETERS: [&str; 7] = [
    ROLE_NAME,
    PROFILE_NAME,
    SESSION_NAME,
    WEB_IDENTITY_TOKEN,
    STS_ENDPOINT,
    EXTERNAL_ID,
    SESSION_TAGS,
];
const CONTENT_LENGTH: &str = "Content-Length";

#[derive(Debug, thiserror::Error)]
pub enum SigningError {
    #[error("signing the request failed")]
    Sign(#[source] SigV4Error),
}

pub struct JsonPost<'call> {
    pub url: &'call str,
    pub region: &'call str,
    pub body: &'call [u8],
    pub headers: &'call Headers,
    pub credentials: &'call Credentials,
    pub now: SystemTime,
}

#[must_use]
pub fn credentials_from_params(litellm_params: &Extra) -> Option<Credentials> {
    warn_unresolved_auth_params(litellm_params);
    let access_key_id = secret_parameter(litellm_params, ACCESS_KEY_ID)?;
    let secret_access_key = secret_parameter(litellm_params, SECRET_ACCESS_KEY)?;
    let session_token = secret_parameter(litellm_params, SESSION_TOKEN);
    Some(Credentials {
        access_key_id,
        secret_access_key,
        session_token,
    })
}

fn warn_unresolved_auth_params(litellm_params: &Extra) {
    for name in UNRESOLVED_AUTH_PARAMETERS {
        let Some(value) = litellm_params.get(name) else {
            continue;
        };
        if matches!(value, Json::Null) {
            continue;
        }
        tracing::error!(
            parameter = name,
            "the AWS auth flow for this credential parameter is not built here; the parameter is ignored"
        );
    }
}

fn secret_parameter(litellm_params: &Extra, name: &'static str) -> Option<SecretString> {
    match litellm_params.get(name) {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(SecretString::from(text.clone())),
        Some(_) => {
            tracing::error!(parameter = name, "the AWS credential parameter is not text");
            None
        }
    }
}

pub fn sign_json_post(request: &JsonPost<'_>) -> Result<Headers, SigningError> {
    let signable = request
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<Vec<(String, String)>>();
    let signed = sign_request(&Signable {
        method: "POST",
        url: request.url,
        headers: &signable,
        body: request.body,
        service: SERVICE,
        region: request.region,
        credentials: request.credentials,
        mode: SigningMode::SigV4Auth,
        now: request.now,
    })
    .map_err(SigningError::Sign)?;
    let mut headers = request.headers.clone();
    for (name, value) in signed {
        headers.insert(name, value);
    }
    headers.insert(CONTENT_LENGTH.to_owned(), request.body.len().to_string());
    Ok(headers)
}
