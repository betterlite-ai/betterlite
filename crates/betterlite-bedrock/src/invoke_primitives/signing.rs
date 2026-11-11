use std::collections::BTreeMap;
use std::future::Future;
use std::time::SystemTime;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::value::RawValue;

use super::config::Params;
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::signing::error::SigningError;
use crate::signing::request::AwsSignedRequest;
use crate::signing::service::{ServiceName, ServiceRequest, sign_service_request};
use crate::signing::target::RegionInputs;

pub struct SignRequest<'text> {
    pub headers: &'text [(String, Option<String>)],
    pub optional_params: &'text Params,
    pub request_data: &'text IndexMap<String, Json>,
    pub api_base: &'text str,
    pub api_key: Option<&'text SecretString>,
    pub configured_bearer: Option<&'text SecretString>,
    pub model: Option<&'text str>,
    // LiteLLM forwards both stream flags to the signer, which ignores them
    pub stream: Option<bool>,
    pub fake_stream: Option<bool>,
    pub region_inputs: RegionInputs<'text>,
    pub now: SystemTime,
}

pub async fn sign_request<F, Fut>(
    request: SignRequest<'_>,
    resolve_credentials: F,
) -> Result<AwsSignedRequest, SigningError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let optional_params: BTreeMap<String, Box<RawValue>> = request
        .optional_params
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    sign_service_request(
        ServiceRequest {
            service: ServiceName::Bedrock,
            headers: request.headers,
            optional_params: &optional_params,
            request_data: request.request_data,
            api_base: request.api_base,
            model: request.model,
            api_key: request.api_key,
            configured_bearer: request.configured_bearer,
            region_inputs: request.region_inputs,
            now: request.now,
        },
        resolve_credentials,
    )
    .await
}
