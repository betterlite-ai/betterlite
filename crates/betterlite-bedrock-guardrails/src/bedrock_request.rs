use std::time::SystemTime;

use betterlite_bedrock::aws_endpoints::runtime::{
    BedrockEndpointType, RuntimeEndpointInput, runtime_endpoint,
};
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::signing::body::Body;
use betterlite_bedrock::signing::endpoint::{RequestHeaders, request_headers};
use betterlite_bedrock::signing::error::SigningError;
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use secrecy::SecretString;
use serde::Serialize;

use crate::bedrock_transport::BedrockGuardrail;

#[derive(Debug, thiserror::Error)]
pub enum PrepareError {
    #[error("the Bedrock request body cannot be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("{field} must be a string")]
    Field { field: &'static str },
    #[error("{field} is required to build the ApplyGuardrail path")]
    MissingIdentity { field: &'static str },
    #[error(transparent)]
    Signing(#[from] SigningError),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RequestEnvironment<'text> {
    pub runtime_endpoint: Option<&'text str>,
    pub bearer_token: Option<&'text SecretString>,
}

#[derive(Debug, Clone, Copy)]
pub struct PrepareInput<'text> {
    pub environment: RequestEnvironment<'text>,
    pub credentials: Option<&'text Credentials>,
    pub region: &'text str,
    pub api_key: Option<&'text SecretString>,
    pub extra_headers: &'text [(String, String)],
    pub request_path: Option<&'text str>,
    pub now: SystemTime,
}

pub fn prepare_request<T>(
    guardrail: &BedrockGuardrail,
    input: PrepareInput<'_>,
    data: &T,
) -> Result<AwsSignedRequest, PrepareError>
where
    T: Serialize,
{
    let url = request_url(guardrail, &input)?;
    let body = serde_json::to_vec(data).map_err(PrepareError::Encode)?;
    let headers = request_header_pairs(input.extra_headers);
    request_headers(RequestHeaders {
        credentials: input.credentials,
        region: input.region,
        extra_headers: input.extra_headers,
        endpoint: &url,
        data: Body::Bytes(&body),
        headers: &headers,
        api_key: input.api_key,
        configured_bearer: input.environment.bearer_token,
        supports_bearer_token: true,
        now: input.now,
    })
    .map_err(PrepareError::from)
}

fn request_url(
    guardrail: &BedrockGuardrail,
    input: &PrepareInput<'_>,
) -> Result<String, PrepareError> {
    let configured = optional_string(&guardrail.optional_params, "aws_bedrock_runtime_endpoint")?;
    let endpoint = runtime_endpoint(RuntimeEndpointInput {
        api_base: None,
        configured_endpoint: configured.as_deref(),
        environment_endpoint: input.environment.runtime_endpoint,
        region: input.region,
        endpoint_type: BedrockEndpointType::Runtime,
    });
    let path = apply_path(guardrail, input.request_path)?;
    let endpoint_url = endpoint.proxy_endpoint_url;
    Ok(format!("{endpoint_url}{path}"))
}

fn apply_path(
    guardrail: &BedrockGuardrail,
    request_path: Option<&str>,
) -> Result<String, PrepareError> {
    let Some(path) = request_path else {
        let identifier =
            guardrail
                .guardrail_identifier
                .as_deref()
                .ok_or(PrepareError::MissingIdentity {
                    field: "guardrailIdentifier",
                })?;
        let version =
            guardrail
                .guardrail_version
                .as_deref()
                .ok_or(PrepareError::MissingIdentity {
                    field: "guardrailVersion",
                })?;
        return Ok(format!("/guardrail/{identifier}/version/{version}/apply"));
    };
    Ok(path.to_owned())
}

fn request_header_pairs(extra_headers: &[(String, String)]) -> Vec<(String, Option<String>)> {
    let mut headers = vec![(
        "Content-Type".to_owned(),
        Some("application/json".to_owned()),
    )];
    for (name, value) in extra_headers {
        match headers.iter_mut().find(|(existing, _)| existing == name) {
            Some((_, stored)) => *stored = Some(value.clone()),
            None => headers.push((name.clone(), Some(value.clone()))),
        }
    }
    headers
}

fn optional_string(
    params: &OpaqueParams,
    field: &'static str,
) -> Result<Option<String>, PrepareError> {
    match params.0.get(field) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(PrepareError::Field { field }),
    }
}
