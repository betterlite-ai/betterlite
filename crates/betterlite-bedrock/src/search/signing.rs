use std::collections::BTreeMap;
use std::fmt::{self, Debug, Formatter};
use std::future::Future;
use std::time::SystemTime;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::bedrock_services_search::{
    GATEWAY_TOKEN_VARIABLE, GATEWAY_URL_VARIABLE,
};
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::search::credentials::ServerCredential;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::{RawValue, to_raw_value};

use super::error::SearchError;
use super::request::Params;
use super::url::{
    credential_safe_transport, gateway_region, is_configured_gateway, signing_region,
};
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::signing::request::AwsSignedRequest;
use crate::signing::service::{ServiceName, ServiceRequest, sign_service_request};
use crate::signing::target::RegionInputs;

const GATEWAY_CREDENTIAL: ServerCredential = ServerCredential {
    key_vars: &[GATEWAY_TOKEN_VARIABLE],
    base_var: Some(GATEWAY_URL_VARIABLE),
    default_base: None,
};

pub struct SignRequest<'search> {
    pub headers: &'search Headers,
    pub optional_params: &'search Params,
    pub request_data: &'search Json,
    pub api_base: &'search str,
    pub api_key: Option<&'search SecretString>,
    pub session_region: Option<&'search str>,
    pub now: SystemTime,
}

// the request carries a caller key and a body, so Debug names the fields and no values
impl Debug for SignRequest<'_> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SignRequest")
            .field("api_base", &self.api_base)
            .field("header_names", &self.headers.keys().collect::<Vec<_>>())
            .field(
                "optional_param_names",
                &self.optional_params.keys().collect::<Vec<_>>(),
            )
            .field("api_key", &self.api_key.map(|_| "<redacted>"))
            .field("request_data", &"<redacted>")
            .finish()
    }
}

pub async fn sign_request<F, Fut>(
    request: &SignRequest<'_>,
    lookup: &dyn EnvironmentLookup,
    resolve_credentials: F,
) -> Result<AwsSignedRequest, SearchError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let Json::Object(body) = request.request_data else {
        return Err(SearchError::RequestBody);
    };
    if !credential_safe_transport(request.api_base) {
        return Err(SearchError::PlaintextTransport {
            api_base: request.api_base.to_owned(),
        });
    }
    let matched_region = gateway_region(request.api_base);
    let caller_base = matched_region.is_none().then_some(request.api_base);
    if let Some(signed) = resolve_bearer(request, body, caller_base, lookup)? {
        return Ok(signed);
    }
    check_signing_host(request.api_base, matched_region.as_deref(), lookup)?;
    let parameters = signing_parameters(request, matched_region.as_deref())?;
    signed_request(request, body, &parameters, resolve_credentials).await
}

fn resolve_bearer(
    request: &SignRequest<'_>,
    body: &Params,
    caller_base: Option<&str>,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<AwsSignedRequest>, SearchError> {
    let Some(token) = GATEWAY_CREDENTIAL.resolve(request.api_key.cloned(), caller_base, lookup)?
    else {
        return Ok(None);
    };
    Ok(Some(bearer_signed(request, body, &token)))
}

fn check_signing_host(
    api_base: &str,
    matched_region: Option<&str>,
    lookup: &dyn EnvironmentLookup,
) -> Result<(), SearchError> {
    let configured = lookup.lookup(GATEWAY_URL_VARIABLE);
    if matched_region.is_some() || is_configured_gateway(api_base, configured.as_deref()) {
        return Ok(());
    }
    Err(SearchError::UntrustedSigningHost {
        api_base: api_base.to_owned(),
    })
}

async fn signed_request<F, Fut>(
    request: &SignRequest<'_>,
    body: &Params,
    parameters: &BTreeMap<String, Box<RawValue>>,
    resolve_credentials: F,
) -> Result<AwsSignedRequest, SearchError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let headers: Vec<(String, Option<String>)> = request
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), Some(value.clone())))
        .collect();
    // api_key="" keeps BaseAWSLLM off the AWS_BEARER_TOKEN_BEDROCK fallback: that token belongs
    // to Bedrock Runtime and must never reach an AgentCore gateway
    let empty = SecretString::from(String::new());
    sign_service_request(
        ServiceRequest {
            service: ServiceName::BedrockAgentcore,
            headers: &headers,
            optional_params: parameters,
            request_data: body,
            api_base: request.api_base,
            model: None,
            api_key: Some(&empty),
            configured_bearer: None,
            region_inputs: RegionInputs::default(),
            now: request.now,
        },
        resolve_credentials,
    )
    .await
}

fn bearer_signed(
    request: &SignRequest<'_>,
    body: &Params,
    token: &SecretString,
) -> AwsSignedRequest {
    let mut headers = request.headers.clone();
    headers.insert(
        "Authorization".to_owned(),
        format!("Bearer {}", token.expose_secret()),
    );
    AwsSignedRequest {
        method: "POST".to_owned(),
        url: request.api_base.to_owned(),
        headers: headers.into_iter().collect(),
        body: dumps_object(body).into_bytes(),
    }
}

fn signing_parameters(
    request: &SignRequest<'_>,
    matched_region: Option<&str>,
) -> Result<BTreeMap<String, Box<RawValue>>, SearchError> {
    let mut parameters = BTreeMap::new();
    for (name, value) in request.optional_params {
        parameters.insert(
            name.clone(),
            to_raw_value(value).map_err(SearchError::Parameters)?,
        );
    }
    let has_explicit_region = request
        .optional_params
        .get("aws_region_name")
        .is_some_and(|value| !matches!(value, Json::Null));
    if !has_explicit_region {
        let region = signing_region(request.api_base, matched_region, request.session_region)?;
        parameters.insert(
            "aws_region_name".to_owned(),
            to_raw_value(&region).map_err(SearchError::Parameters)?,
        );
    }
    Ok(parameters)
}
