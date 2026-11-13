use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::time::SystemTime;

use aws_credential_types::Credentials as SigningCredentials;
use aws_sigv4::http_request::{
    PayloadChecksumKind, PercentEncodingMode, SessionTokenMode, SignableBody, SignableRequest,
    SignatureLocation, SigningParams, SigningSettings, UriPathNormalizationMode, sign,
};
use aws_sigv4::sign::v4;
use aws_smithy_runtime_api::client::identity::Identity;
use secrecy::{ExposeSecret as _, SecretString};

use crate::s3_transport::error::S3TransportError;

mod tests;

const SERVICE: &str = "s3";

#[derive(Clone, Debug)]
pub struct Credentials {
    pub access_key_id: SecretString,
    pub secret_access_key: SecretString,
    pub session_token: Option<SecretString>,
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("AWS credentials for S3 could not be resolved")]
    Unavailable,
}

pub trait CredentialSource: Send + Sync + Debug {
    fn resolve(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Credentials, CredentialError>> + Send + '_>>;
}

#[derive(Clone, Debug)]
pub struct StaticCredentials {
    pub credentials: Credentials,
}

impl CredentialSource for StaticCredentials {
    fn resolve(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Credentials, CredentialError>> + Send + '_>> {
        let credentials = self.credentials.clone();
        Box::pin(async move { Ok(credentials) })
    }
}

pub trait Clock: Send + Sync + Debug {
    fn now(&self) -> SystemTime;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RequestToSign<'text> {
    pub method: &'text str,
    pub url: &'text str,
    pub headers: &'text [(String, String)],
    pub content_sha256: &'text str,
    pub region: &'text str,
    pub now: SystemTime,
}

pub fn sign_request(
    request: &RequestToSign<'_>,
    credentials: &Credentials,
) -> Result<Vec<(String, String)>, S3TransportError> {
    let identity = Identity::from(SigningCredentials::new(
        credentials.access_key_id.expose_secret().to_owned(),
        credentials.secret_access_key.expose_secret().to_owned(),
        credentials
            .session_token
            .as_ref()
            .map(|token| token.expose_secret().to_owned()),
        None,
        "betterlite-logging",
    ));
    let mut settings = SigningSettings::default();
    // botocore's S3SigV4Auth leaves the path as sent, so the canonical form must not normalize it
    settings.uri_path_normalization_mode = UriPathNormalizationMode::Disabled;
    settings.percent_encoding_mode = PercentEncodingMode::Single;
    settings.payload_checksum_kind = PayloadChecksumKind::NoHeader;
    settings.signature_location = SignatureLocation::Headers;
    settings.session_token_mode = SessionTokenMode::Include;
    let parameters = v4::SigningParams::builder()
        .identity(&identity)
        .region(request.region)
        .name(SERVICE)
        .time(request.now)
        .settings(settings)
        .build()?;
    let signed = request
        .headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()));
    let signable = SignableRequest::new(
        request.method,
        request.url,
        signed,
        SignableBody::Precomputed(request.content_sha256.to_owned()),
    )?;
    let output = sign(signable, &SigningParams::from(parameters))?;
    let (instructions, _signature) = output.into_parts();
    let (headers, _query) = instructions.into_parts();
    Ok(headers
        .into_iter()
        .map(|header| (header.name().to_owned(), header.value().to_owned()))
        .collect())
}
