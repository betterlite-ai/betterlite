use std::collections::BTreeMap;
use std::future::Future;
use std::time::SystemTime;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::value::RawValue;

use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::signing::request::AwsSignedRequest;
use crate::signing::service::{ServiceName, ServiceRequest, sign_service_request};
use crate::signing::target::RegionInputs;
use crate::vector_stores::error::VectorStoreError;

pub struct SignRequest<'request> {
    pub headers: &'request [(String, Option<String>)],
    pub optional_params: &'request BTreeMap<String, Box<RawValue>>,
    pub request_data: &'request IndexMap<String, Json>,
    pub api_base: &'request str,
    pub api_key: Option<&'request SecretString>,
    pub configured_bearer: Option<&'request SecretString>,
    pub region_inputs: RegionInputs<'request>,
    pub now: SystemTime,
}

pub async fn sign_request<F, Fut>(
    request: SignRequest<'_>,
    resolve_credentials: F,
) -> Result<AwsSignedRequest, VectorStoreError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    Ok(sign_service_request(
        ServiceRequest {
            service: ServiceName::Bedrock,
            headers: request.headers,
            optional_params: request.optional_params,
            request_data: request.request_data,
            api_base: request.api_base,
            model: None,
            api_key: request.api_key,
            configured_bearer: request.configured_bearer,
            region_inputs: request.region_inputs,
            now: request.now,
        },
        resolve_credentials,
    )
    .await?)
}
