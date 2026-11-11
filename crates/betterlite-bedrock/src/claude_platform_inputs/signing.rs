use std::collections::BTreeMap;
use std::future::Future;
use std::time::SystemTime;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;

use super::error::PlatformInputError;
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::signing::request::AwsSignedRequest;
use crate::signing::service::{ServiceName, ServiceRequest, sign_service_request};
use crate::signing::target::RegionInputs;

pub struct PlatformSignRequest<'source> {
    pub headers: &'source [(String, Option<String>)],
    pub optional_params: &'source BTreeMap<String, Box<RawValue>>,
    pub request_data: &'source IndexMap<String, Json>,
    pub api_base: &'source str,
    pub api_key: Option<&'source SecretString>,
    pub environment_api_key: Option<&'source SecretString>,
    pub configured_bearer: Option<&'source SecretString>,
    pub model: Option<&'source str>,
    pub region_inputs: RegionInputs<'source>,
    pub now: SystemTime,
}

#[derive(Debug)]
pub enum PlatformSignature {
    Supplied {
        headers: Vec<(String, Option<String>)>,
    },
    Signed(AwsSignedRequest),
}

pub async fn sign_platform_request<F, Fut>(
    request: PlatformSignRequest<'_>,
    resolve_credentials: F,
) -> Result<PlatformSignature, PlatformInputError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let supplied = [request.api_key, request.environment_api_key]
        .into_iter()
        .flatten()
        .any(|value| !value.expose_secret().is_empty());
    if supplied {
        return Ok(PlatformSignature::Supplied {
            headers: request.headers.to_vec(),
        });
    }
    let signed = sign_service_request(
        ServiceRequest {
            service: ServiceName::AwsExternalAnthropic,
            headers: request.headers,
            optional_params: request.optional_params,
            request_data: request.request_data,
            api_base: request.api_base,
            model: request.model,
            api_key: None,
            configured_bearer: request.configured_bearer,
            region_inputs: request.region_inputs,
            now: request.now,
        },
        resolve_credentials,
    )
    .await?;
    Ok(PlatformSignature::Signed(signed))
}
