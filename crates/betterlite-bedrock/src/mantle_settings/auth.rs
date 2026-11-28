use std::collections::BTreeMap;
use std::future::Future;
use std::time::SystemTime;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::{RawValue, to_raw_value};

use crate::aws_settings::or_bearer_token;
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::mantle_inputs::records::MantleGlobals;
use crate::mantle_inputs::region::{region_from_host, resolve_region};
use crate::signing::error::SigningError;
use crate::signing::request::AwsSignedRequest;
use crate::signing::service::{ServiceName, ServiceRequest, sign_service_request};
use crate::signing::target::RegionInputs;

use super::error::MantleSettingsError;
use super::field_text;

pub const AUTHORIZATION: &str = "Authorization";
pub const REGION_PARAM: &str = "aws_region_name";

#[derive(Clone, Copy)]
pub struct SignRequest<'text> {
    pub headers: &'text [(String, Option<String>)],
    pub optional_params: &'text Object,
    pub request_data: &'text IndexMap<String, Json>,
    pub api_base: &'text str,
    pub api_key: Option<&'text SecretString>,
    pub model: Option<&'text str>,
    pub stream: Option<bool>,
    pub fake_stream: Option<bool>,
    pub globals: &'text MantleGlobals,
    pub now: SystemTime,
}

// BedrockMantleAuthMixin.sign_request: a Bearer token wins and leaves headers and options untouched; without one the signing region comes from the actual URL host before the shared chain, options carry that region, and every caller Authorization header is dropped.
pub async fn sign_request<Resolve, Fut>(
    request: SignRequest<'_>,
    resolve_credentials: Resolve,
) -> Result<AwsSignedRequest, MantleSettingsError>
where
    Resolve: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let bearer = or_bearer_token(&[
        request.api_key,
        request.globals.mantle_api_key.as_ref(),
        request.globals.aws_bearer_token.as_ref(),
    ])
    .filter(|token| !token.expose_secret().is_empty());
    let (params, filtered, token) = match bearer {
        Some(token) => (raw_params(request.optional_params)?, None, Some(token)),
        None => (
            sigv4_params(request)?,
            Some(without_authorization(request.headers)),
            None,
        ),
    };
    let headers = filtered.as_deref().unwrap_or(request.headers);
    sign(
        request,
        &params,
        headers,
        token.as_ref(),
        resolve_credentials,
    )
    .await
}

fn sigv4_params(
    request: SignRequest<'_>,
) -> Result<BTreeMap<String, Box<RawValue>>, MantleSettingsError> {
    let region = signing_region(request.optional_params, request.api_base, request.globals)?;
    let mut params = raw_params(request.optional_params)?;
    params.insert(
        REGION_PARAM.to_owned(),
        to_raw_value(&region).map_err(MantleSettingsError::Encode)?,
    );
    Ok(params)
}

fn without_authorization(headers: &[(String, Option<String>)]) -> Vec<(String, Option<String>)> {
    headers
        .iter()
        .filter(|(name, _)| !name.eq_ignore_ascii_case(AUTHORIZATION))
        .cloned()
        .collect()
}

async fn sign<Resolve, Fut>(
    request: SignRequest<'_>,
    params: &BTreeMap<String, Box<RawValue>>,
    headers: &[(String, Option<String>)],
    api_key: Option<&SecretString>,
    resolve_credentials: Resolve,
) -> Result<AwsSignedRequest, MantleSettingsError>
where
    Resolve: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    sign_service_request(
        ServiceRequest {
            service: ServiceName::Bedrock,
            headers,
            optional_params: params,
            request_data: request.request_data,
            api_base: request.api_base,
            model: request.model,
            api_key,
            configured_bearer: None,
            region_inputs: RegionInputs::default(),
            now: request.now,
        },
        resolve_credentials,
    )
    .await
    .map_err(classify)
}

// The four botocore exceptions LiteLLM maps to its Mantle ValueError are the credential-source failures; every other signing failure passes through.
fn classify(error: SigningError) -> MantleSettingsError {
    if is_credential_failure(&error) {
        return MantleSettingsError::Auth {
            source: Box::new(error),
        };
    }
    MantleSettingsError::Signing(error)
}

const fn is_credential_failure(error: &SigningError) -> bool {
    matches!(
        error,
        SigningError::Credential(CredentialError::Source(_) | CredentialError::MissingCredentials)
    )
}

// A recognized Mantle host pins the credential scope to the host's own region, even when a stale aws_region_name says otherwise; region-selection failures happen before the signer runs.
fn signing_region(
    params: &Object,
    api_base: &str,
    globals: &MantleGlobals,
) -> Result<String, MantleSettingsError> {
    if let Some(region) = region_from_host(api_base) {
        return Ok(region);
    }
    let explicit = field_text(params, REGION_PARAM)?;
    resolve_region(explicit.as_deref(), Some(api_base), globals).map_err(MantleSettingsError::from)
}

fn raw_params(params: &Object) -> Result<BTreeMap<String, Box<RawValue>>, MantleSettingsError> {
    let mut raw = BTreeMap::new();
    for (name, value) in params {
        raw.insert(
            name.clone(),
            to_raw_value(value).map_err(MantleSettingsError::Encode)?,
        );
    }
    Ok(raw)
}
