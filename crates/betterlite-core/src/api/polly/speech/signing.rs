use std::time::SystemTime;

use aws_credential_types::Credentials;
use aws_sigv4::http_request::{
    SignableBody, SignableRequest, SigningParams, SigningSettings, sign,
};
use aws_sigv4::sign::v4;
use aws_smithy_runtime_api::client::identity::Identity;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use super::credentials::ResolvedCredentials;
use super::error::SpeechError;

pub const SERVICE: &str = "polly";
const PROVIDER_NAME: &str = "betterlite";
const CONTENT_TYPE_NAME: &str = "Content-Type";
const CONTENT_TYPE_VALUE: &str = "application/json";

pub struct SignablePost<'text> {
    pub url: &'text str,
    pub region: &'text str,
    pub body: &'text [u8],
    pub now: SystemTime,
    pub credentials: &'text ResolvedCredentials,
}

// LiteLLM signs a POST over service=polly with only Content-Type set, and the bytes it signs are the body it sends
pub fn sign_post(request: &SignablePost<'_>) -> Result<IndexMap<String, String>, SpeechError> {
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
    let identity = Identity::new(credentials, None);
    let parameters = v4::SigningParams::builder()
        .identity(&identity)
        .region(request.region)
        .name(SERVICE)
        .time(request.now)
        .settings(SigningSettings::default())
        .build()
        .map_err(|source| SpeechError::SigningParameters(Box::new(source)))?;
    let mut headers = IndexMap::new();
    headers.insert(CONTENT_TYPE_NAME.to_owned(), CONTENT_TYPE_VALUE.to_owned());
    let signable = SignableRequest::new(
        "POST",
        request.url,
        headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
        SignableBody::Bytes(request.body),
    )
    .map_err(|source| SpeechError::Signable(Box::new(source)))?;
    let (instructions, _signature) = sign(signable, &SigningParams::from(parameters))
        .map_err(|source| SpeechError::Sign(Box::new(source)))?
        .into_parts();
    for (name, value) in instructions.headers() {
        headers.insert(litellm_casing(name).to_owned(), value.to_owned());
    }
    Ok(headers)
}

// the signature is case-insensitive, but botocore returns these names as LiteLLM hands them on
fn litellm_casing(name: &str) -> &str {
    match name {
        "authorization" => "Authorization",
        "x-amz-date" => "X-Amz-Date",
        "x-amz-security-token" => "X-Amz-Security-Token",
        other => other,
    }
}
