use std::collections::BTreeMap;
use std::future::Future;
use std::time::SystemTime;

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_bedrock::signing::service::{ServiceName, ServiceRequest, sign_service_request};
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::value::RawValue;

use super::error::S3VectorsSearchError;

pub struct SignSearch<'request> {
    pub headers: &'request [(String, Option<String>)],
    pub optional_params: &'request BTreeMap<String, Box<RawValue>>,
    pub request_data: &'request IndexMap<String, Json>,
    pub api_base: &'request str,
    pub api_key: Option<&'request SecretString>,
    pub configured_bearer: Option<&'request SecretString>,
    pub region_inputs: RegionInputs<'request>,
    pub now: SystemTime,
}

pub async fn sign_search_request<F, Fut>(
    request: SignSearch<'_>,
    resolve_credentials: F,
) -> Result<AwsSignedRequest, S3VectorsSearchError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    Ok(sign_service_request(
        ServiceRequest {
            service: ServiceName::S3vectors,
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
