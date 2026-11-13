use super::error::SigningError;
use super::headers::{merge_computed, prepare, without_computed};
use super::request::AwsSignedRequest;
use super::sigv4::{Signable, sign_request};
use crate::aws_settings::sources::Credentials;
use std::time::SystemTime;
#[derive(Debug, Clone, Copy)]
pub struct JsonPost<'source> {
    pub service_name: &'source str,
    pub region: Option<&'source str>,
    pub url: &'source str,
    pub body: &'source str,
    pub headers: &'source [(String, String)],
    pub credentials: &'source Credentials,
    pub now: SystemTime,
}
pub fn sign_aws_json_post(request: JsonPost<'_>) -> Result<AwsSignedRequest, SigningError> {
    let Some(region) = request.region else {
        return Err(SigningError::MissingRegion);
    };
    let has_token = request.credentials.session_token.is_some();
    let supplied = without_computed(request.headers.to_vec(), has_token);
    let computed = sign_request(&Signable {
        method: "POST",
        url: request.url,
        headers: &supplied,
        body: request.body.as_bytes(),
        service: request.service_name,
        region,
        credentials: request.credentials,
        now: request.now,
    })?;
    let headers = merge_computed(supplied, computed);
    Ok(AwsSignedRequest {
        method: "POST".to_owned(),
        url: request.url.to_owned(),
        headers: prepare(headers, request.body.chars().count(), "POST"),
        body: request.body.as_bytes().to_vec(),
    })
}
