use std::time::SystemTime;

use aws_credential_types::Credentials;
use aws_sigv4::http_request::{
    SignableBody, SignableRequest, SigningError as AwsSigningError, SigningParams, SigningSettings,
    sign,
};
use aws_sigv4::sign::v4;
use aws_sigv4::sign::v4::signing_params::BuildError;
use aws_smithy_runtime_api::client::identity::Identity;
use betterlite_models::decisions::{Extra, Json};
use secrecy::{ExposeSecret as _, SecretString};

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
const PROVIDER_NAME: &str = "betterlite";
const CONTENT_LENGTH: &str = "Content-Length";

#[derive(Clone)]
pub struct AwsCredentials {
    pub access_key_id: SecretString,
    pub secret_access_key: SecretString,
    pub session_token: Option<SecretString>,
}

#[derive(Debug, thiserror::Error)]
pub enum SigningError {
    #[error("the signing parameters are invalid")]
    Parameters(#[source] Box<BuildError>),
    #[error("the request cannot be signed")]
    Signable(#[source] Box<AwsSigningError>),
    #[error("signing the request failed")]
    Sign(#[source] Box<AwsSigningError>),
}

pub struct JsonPost<'call> {
    pub url: &'call str,
    pub region: &'call str,
    pub body: &'call [u8],
    pub headers: &'call Headers,
    pub credentials: &'call AwsCredentials,
    pub now: SystemTime,
}

#[must_use]
pub fn credentials_from_params(litellm_params: &Extra) -> Option<AwsCredentials> {
    warn_unresolved_auth_params(litellm_params);
    let access_key_id = secret_parameter(litellm_params, ACCESS_KEY_ID)?;
    let secret_access_key = secret_parameter(litellm_params, SECRET_ACCESS_KEY)?;
    let session_token = secret_parameter(litellm_params, SESSION_TOKEN);
    Some(AwsCredentials {
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
    let credentials = Credentials::new(
        request.credentials.access_key_id.expose_secret(),
        request.credentials.secret_access_key.expose_secret(),
        request
            .credentials
            .session_token
            .as_ref()
            .map(|token| token.expose_secret().to_owned()),
        None,
        PROVIDER_NAME,
    );
    let identity = Identity::from(credentials);
    let parameters = v4::SigningParams::builder()
        .identity(&identity)
        .region(request.region)
        .name(SERVICE)
        .time(request.now)
        .settings(SigningSettings::default())
        .build()
        .map_err(|source| SigningError::Parameters(Box::new(source)))?;
    let signable = SignableRequest::new(
        "POST",
        request.url,
        request
            .headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
        SignableBody::Bytes(request.body),
    )
    .map_err(|source| SigningError::Signable(Box::new(source)))?;
    let (instructions, _signature) = sign(signable, &SigningParams::from(parameters))
        .map_err(|source| SigningError::Sign(Box::new(source)))?
        .into_parts();
    let mut signed = request.headers.clone();
    for (name, value) in instructions.headers() {
        signed.insert(litellm_casing(name).to_owned(), value.to_owned());
    }
    signed.insert(CONTENT_LENGTH.to_owned(), request.body.len().to_string());
    Ok(signed)
}

// the signature itself is case-insensitive, but LiteLLM returns these names as botocore spells them
fn litellm_casing(name: &str) -> &str {
    match name {
        "authorization" => "Authorization",
        "x-amz-date" => "X-Amz-Date",
        "x-amz-security-token" => "X-Amz-Security-Token",
        other => other,
    }
}
