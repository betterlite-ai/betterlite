use std::collections::BTreeMap;
use std::future::Future;
use std::time::SystemTime;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;

use crate::aws_settings::or_bearer_token;
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::responses::error::ResponsesError;
use crate::signing::service::{ServiceName, ServiceRequest, sign_service_request};
use crate::signing::target::RegionInputs;

#[derive(Clone, Copy)]
pub struct SignInputs<'request> {
    pub headers: &'request [(String, Option<String>)],
    pub optional_params: &'request BTreeMap<String, Box<RawValue>>,
    pub request_data: &'request IndexMap<String, Json>,
    pub api_base: &'request str,
    pub model: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub environment_token: Option<&'request SecretString>,
    pub region_inputs: RegionInputs<'request>,
    pub now: SystemTime,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SignedHeaders {
    pub headers: Vec<(String, Option<String>)>,
    pub body: Option<Vec<u8>>,
}

pub fn validate_environment(
    headers: &mut IndexMap<String, String>,
    api_key: Option<&SecretString>,
    environment_token: Option<&SecretString>,
) {
    let bearer = or_bearer_token(&[api_key, environment_token]);
    if let Some(token) = bearer.filter(|value| !value.expose_secret().is_empty()) {
        headers.insert(
            "Authorization".to_owned(),
            format!("Bearer {}", token.expose_secret()),
        );
    }
}

pub async fn sign_request<F, Fut>(
    inputs: SignInputs<'_>,
    resolve_credentials: F,
) -> Result<SignedHeaders, ResponsesError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let bearer = or_bearer_token(&[inputs.api_key, inputs.environment_token]);
    if bearer.is_some_and(|token| !token.expose_secret().is_empty()) {
        return Ok(SignedHeaders {
            headers: inputs.headers.to_vec(),
            body: None,
        });
    }
    let signed = sign_service_request(
        ServiceRequest {
            service: ServiceName::Bedrock,
            headers: inputs.headers,
            optional_params: inputs.optional_params,
            request_data: inputs.request_data,
            api_base: inputs.api_base,
            model: inputs.model,
            api_key: None,
            configured_bearer: None,
            region_inputs: inputs.region_inputs,
            now: inputs.now,
        },
        resolve_credentials,
    )
    .await?;
    Ok(SignedHeaders {
        headers: signed
            .headers
            .into_iter()
            .map(|(name, value)| (name, Some(value)))
            .collect(),
        body: Some(signed.body),
    })
}
