use std::collections::BTreeMap;
use std::future::Future;
use std::time::SystemTime;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::{RawValue, to_raw_value};

use super::error::AgentCoreRequestError;
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::model_routes::{ParamMap, ParamValue};
use crate::signing::headers::{resolved, set_exact};
use crate::signing::request::AwsSignedRequest;
use crate::signing::service::{ServiceName, ServiceRequest, sign_service_request};
use crate::signing::target::RegionInputs;

const ACCEPT: &str = "application/json, text/event-stream";

pub struct SignRequest<'text> {
    pub headers: &'text [(String, Option<String>)],
    pub optional_params: &'text ParamMap,
    pub request_data: &'text IndexMap<String, Json>,
    pub api_base: &'text str,
    pub model: Option<&'text str>,
    pub api_key: Option<&'text SecretString>,
    pub configured_bearer: Option<&'text SecretString>,
    pub region_inputs: RegionInputs<'text>,
    pub now: SystemTime,
}

pub async fn sign_request<F, Fut>(
    request: SignRequest<'_>,
    resolve_credentials: F,
) -> Result<AwsSignedRequest, AgentCoreRequestError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let mut headers = request.headers.to_vec();
    set_exact(&mut headers, "Accept", ACCEPT.to_owned());
    if let Some(token) = selected_jwt(request.api_key, request.optional_params)? {
        return Ok(bearer_signed(&request, &headers, &token));
    }
    tracing::debug!(authentication = "sigv4", "AgentCore request authentication");
    sigv4_signed(request, &headers, resolve_credentials).await
}

fn bearer_signed(
    request: &SignRequest<'_>,
    supplied: &[(String, Option<String>)],
    token: &SecretString,
) -> AwsSignedRequest {
    tracing::debug!(
        authentication = "bearer",
        "AgentCore request authentication"
    );
    let mut headers = supplied.to_vec();
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

async fn sigv4_signed<F, Fut>(
    request: SignRequest<'_>,
    headers: &[(String, Option<String>)],
    resolve_credentials: F,
) -> Result<AwsSignedRequest, AgentCoreRequestError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let parameters = raw_parameters(request.optional_params)?;
    Ok(sign_service_request(
        ServiceRequest {
            service: ServiceName::BedrockAgentcore,
            headers,
            optional_params: &parameters,
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
    .await?)
}

// LiteLLM takes the explicit api_key when it is truthy, then a truthy optional_params["api_key"]
fn selected_jwt<'text>(
    api_key: Option<&'text SecretString>,
    optional_params: &'text ParamMap,
) -> Result<Option<SecretString>, AgentCoreRequestError> {
    if let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) {
        return Ok(Some(key.clone()));
    }
    match optional_params.get("api_key") {
        None | Some(ParamValue::Null) => Ok(None),
        Some(ParamValue::Text(text)) if text.is_empty() => Ok(None),
        Some(ParamValue::Text(text)) => Ok(Some(SecretString::from(text.clone()))),
        Some(_) => Err(AgentCoreRequestError::WrongType { field: "api_key" }),
    }
}

fn raw_parameters(
    optional_params: &ParamMap,
) -> Result<BTreeMap<String, Box<RawValue>>, AgentCoreRequestError> {
    let mut raw = BTreeMap::new();
    for (name, value) in optional_params {
        let encoded =
            to_raw_value(value).map_err(|source| AgentCoreRequestError::Parameters { source })?;
        raw.insert(name.clone(), encoded);
    }
    Ok(raw)
}
