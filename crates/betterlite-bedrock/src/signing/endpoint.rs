use super::body::Body;
use super::error::SigningError;
use super::headers::{
    filter_for_signature, prepare, reattach, replace, resolved, set_exact, without_computed,
};
use super::request::AwsSignedRequest;
use super::sigv4::{Signable, sign_request};
use crate::aws_settings::bedrock_bearer_token;
use crate::aws_settings::sources::Credentials;
use secrecy::{ExposeSecret as _, SecretString};
use std::time::SystemTime;
#[derive(Debug, Clone, Copy)]
pub struct RequestHeaders<'source> {
    pub credentials: Option<&'source Credentials>,
    pub region: &'source str,
    pub extra_headers: &'source [(String, String)],
    pub endpoint: &'source str,
    pub data: Body<'source>,
    pub headers: &'source [(String, Option<String>)],
    pub api_key: Option<&'source SecretString>,
    pub configured_bearer: Option<&'source SecretString>,
    pub supports_bearer_token: bool,
    pub now: SystemTime,
}
pub fn request_headers(request: RequestHeaders<'_>) -> Result<AwsSignedRequest, SigningError> {
    let bearer = if request.supports_bearer_token {
        bedrock_bearer_token(request.api_key, request.configured_bearer)
    } else {
        None
    };
    if let Some(token) = bearer {
        return Ok(bearer_signed(request, &token));
    }
    let Some(credentials) = request.credentials else {
        return Err(SigningError::NoCredentials);
    };
    let has_token = credentials.session_token.is_some();
    let signature = without_computed(filter_for_signature(request.headers), has_token);
    let computed = sign_request(&Signable {
        method: "POST",
        url: request.endpoint,
        headers: &signature,
        body: request.data.bytes(),
        service: "bedrock",
        region: request.region,
        credentials,
        now: request.now,
    })?;
    let mut headers = computed;
    reattach(request.headers, &mut headers, has_token);
    if let Some(value) = extra_authorization(request.extra_headers) {
        replace(&mut headers, "Authorization".to_owned(), value.to_owned());
    }
    Ok(AwsSignedRequest {
        method: "POST".to_owned(),
        url: request.endpoint.to_owned(),
        headers: prepare(headers, request.data.content_length(), "POST"),
        body: request.data.bytes().to_vec(),
    })
}
fn bearer_signed(request: RequestHeaders<'_>, token: &SecretString) -> AwsSignedRequest {
    let mut headers = request.headers.to_vec();
    set_exact(
        &mut headers,
        "Authorization",
        format!("Bearer {}", token.expose_secret()),
    );
    AwsSignedRequest {
        method: "POST".to_owned(),
        url: request.endpoint.to_owned(),
        headers: prepare(resolved(&headers), request.data.content_length(), "POST"),
        body: request.data.bytes().to_vec(),
    }
}
fn extra_authorization(extra: &[(String, String)]) -> Option<&str> {
    let (_, value) = extra.iter().find(|(name, _)| name == "Authorization")?;
    if value.starts_with("AWS4-HMAC-SHA256") {
        return None;
    }
    Some(value.as_str())
}
