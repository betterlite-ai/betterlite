use std::time::SystemTime;

use aws_credential_types::Credentials;
use aws_sigv4::http_request::{
    SignableBody, SignableRequest, SigningError as AwsSigningError, SigningParams, SigningSettings,
    sign,
};
use aws_sigv4::sign::v4;
use aws_sigv4::sign::v4::signing_params::BuildError;
use aws_smithy_runtime_api::client::identity::Identity;
use thiserror::Error;

mod tests;

pub const SERVICE: &str = "dynamodb";

#[derive(Debug, Error)]
pub enum SigningError {
    #[error("the DynamoDB signing parameters are invalid")]
    Parameters(#[from] BuildError),
    #[error("the DynamoDB request could not be signed")]
    Signing(#[from] AwsSigningError),
}

pub struct Signable<'text> {
    pub method: &'text str,
    pub url: &'text str,
    pub headers: &'text [(String, String)],
    pub body: &'text [u8],
    pub region: &'text str,
    pub now: SystemTime,
}

pub fn signed_headers(
    request: &Signable<'_>,
    credentials: &Credentials,
) -> Result<Vec<(String, String)>, SigningError> {
    let signable = SignableRequest::new(
        request.method,
        request.url,
        request
            .headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
        SignableBody::Bytes(request.body),
    )?;
    let identity = Identity::from(credentials.clone());
    let parameters = v4::SigningParams::builder()
        .identity(&identity)
        .region(request.region)
        .name(SERVICE)
        .time(request.now)
        .settings(SigningSettings::default())
        .build()?;
    let signed = sign(signable, &SigningParams::from(parameters))?;
    let (instructions, _signature) = signed.into_parts();
    let (headers, _query) = instructions.into_parts();
    Ok(headers
        .into_iter()
        .map(|header| (header.name().to_owned(), header.value().to_owned()))
        .collect())
}
