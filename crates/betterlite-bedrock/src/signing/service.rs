use super::error::SigningError;
use super::headers::{
    filter_for_signature, override_authorization, reattach, resolved, set_exact, without_computed,
};
use super::request::AwsSignedRequest;
use super::sigv4::{Signable, sign_request};
use super::target::{RegionInputs, auth_settings, resolve_region};
use crate::aws_settings::bedrock_bearer_token;
use crate::aws_settings::pop_aws_auth_params;
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::future::Future;
use std::time::SystemTime;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceName {
    Bedrock,
    Sagemaker,
    BedrockAgentcore,
    S3vectors,
    AwsExternalAnthropic,
}
impl ServiceName {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bedrock => "bedrock",
            Self::Sagemaker => "sagemaker",
            Self::BedrockAgentcore => "bedrock-agentcore",
            Self::S3vectors => "s3vectors",
            Self::AwsExternalAnthropic => "aws-external-anthropic",
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct ServiceRequest<'source> {
    pub service: ServiceName,
    pub headers: &'source [(String, Option<String>)],
    pub optional_params: &'source BTreeMap<String, Box<RawValue>>,
    pub request_data: &'source IndexMap<String, Json>,
    pub api_base: &'source str,
    pub model: Option<&'source str>,
    pub api_key: Option<&'source SecretString>,
    pub configured_bearer: Option<&'source SecretString>,
    pub region_inputs: RegionInputs<'source>,
    pub now: SystemTime,
}
pub async fn sign_service_request<F, Fut>(
    request: ServiceRequest<'_>,
    resolve_credentials: F,
) -> Result<AwsSignedRequest, SigningError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    if let Some(token) = bedrock_bearer_token(request.api_key, request.configured_bearer) {
        return Ok(bearer_signed(request, &token));
    }
    let mut copy = request.optional_params.clone();
    let auth = pop_aws_auth_params(&mut copy).map_err(SigningError::Auth)?;
    let region = resolve_region(
        request.optional_params,
        request.model,
        request.region_inputs,
    )?;
    let settings = auth_settings(auth, &region);
    let credentials = resolve_credentials(&settings, &region)
        .await
        .map_err(SigningError::Credential)?;
    sigv4_signed(request, &credentials, &region)
}
fn bearer_signed(request: ServiceRequest<'_>, token: &SecretString) -> AwsSignedRequest {
    let mut headers = request.headers.to_vec();
    set_exact(&mut headers, "Content-Type", "application/json".to_owned());
    set_exact(
        &mut headers,
        "Authorization",
        format!("Bearer {}", token.expose_secret()),
    );
    AwsSignedRequest {
        method: "POST".to_owned(),
        url: request.api_base.to_owned(),
        headers: resolved(&headers),
        body: dumps_object(request.request_data).into_bytes(),
    }
}
fn sigv4_signed(
    request: ServiceRequest<'_>,
    credentials: &Credentials,
    region: &str,
) -> Result<AwsSignedRequest, SigningError> {
    let mut headers = request.headers.to_vec();
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
    {
        headers.insert(
            0,
            (
                "Content-Type".to_owned(),
                Some("application/json".to_owned()),
            ),
        );
    }
    let has_token = credentials.session_token.is_some();
    let signature = without_computed(filter_for_signature(&headers), has_token);
    let body = dumps_object(request.request_data).into_bytes();
    let mut signed = sign_request(&Signable {
        method: "POST",
        url: request.api_base,
        headers: &signature,
        body: &body,
        service: request.service.as_str(),
        region,
        credentials,
        now: request.now,
    })?;
    reattach(&headers, &mut signed, has_token);
    let supplied = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .find_map(|(_, value)| value.as_deref());
    override_authorization(&mut signed, supplied);
    Ok(AwsSignedRequest {
        method: "POST".to_owned(),
        url: request.api_base.to_owned(),
        headers: signed,
        body,
    })
}
