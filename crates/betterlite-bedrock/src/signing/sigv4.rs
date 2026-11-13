use super::error::SigningError;
use crate::aws_settings::sources::Credentials;
use aws_credential_types::Credentials as SigV4Credentials;
use aws_sigv4::http_request::{
    SignableBody, SignableRequest, SigningParams, SigningSettings, sign,
};
use aws_sigv4::sign::v4;
use aws_smithy_runtime_api::client::identity::Identity;
use secrecy::ExposeSecret as _;
use std::time::SystemTime;
#[derive(Debug, Clone, Copy)]
pub struct Signable<'source> {
    pub method: &'source str,
    pub url: &'source str,
    pub headers: &'source [(String, String)],
    pub body: &'source [u8],
    pub service: &'source str,
    pub region: &'source str,
    pub credentials: &'source Credentials,
    pub now: SystemTime,
}
pub fn sign_request(request: &Signable<'_>) -> Result<Vec<(String, String)>, SigningError> {
    let identity = identity(request.credentials);
    let parameters = v4::SigningParams::builder()
        .identity(&identity)
        .region(request.region)
        .name(request.service)
        .time(request.now)
        .settings(SigningSettings::default())
        .build()
        .map_err(SigningError::Parameters)?;
    let signed = request.headers.iter().filter(|(name, _)| !unsigned(name));
    let signable = SignableRequest::new(
        request.method,
        request.url,
        signed.map(|(name, value)| (name.as_str(), value.as_str())),
        payload(request.headers, request.body),
    )
    .map_err(SigningError::Signable)?;
    let output =
        sign(signable, &SigningParams::from(parameters)).map_err(SigningError::Signable)?;
    let (instructions, _signature) = output.into_parts();
    let (headers, _query) = instructions.into_parts();
    Ok(headers
        .into_iter()
        .map(|header| (botocore_name(header.name()), header.value().to_owned()))
        .collect())
}
// botocore never signs these headers
fn unsigned(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "baggage"
            | "connection"
            | "expect"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "traceparent"
            | "tracestate"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "user-agent"
            | "x-amzn-trace-id"
    )
}
fn identity(credentials: &Credentials) -> Identity {
    Identity::from(SigV4Credentials::new(
        credentials.access_key_id.expose_secret(),
        credentials.secret_access_key.expose_secret(),
        credentials
            .session_token
            .as_ref()
            .map(|token| token.expose_secret().to_owned()),
        None,
        "betterlite-bedrock",
    ))
}
// botocore signs with a caller-supplied X-Amz-Content-SHA256 instead of hashing the body
fn payload<'body>(headers: &[(String, String)], body: &'body [u8]) -> SignableBody<'body> {
    match headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("x-amz-content-sha256"))
    {
        Some((_, value)) => SignableBody::Precomputed(value.clone()),
        None => SignableBody::Bytes(body),
    }
}
fn botocore_name(name: &str) -> String {
    match name {
        "authorization" => "Authorization".to_owned(),
        "x-amz-date" => "X-Amz-Date".to_owned(),
        "x-amz-security-token" => "X-Amz-Security-Token".to_owned(),
        other => other.to_owned(),
    }
}
