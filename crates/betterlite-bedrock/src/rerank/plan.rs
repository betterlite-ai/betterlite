use std::collections::BTreeMap;
use std::future::Future;
use std::time::{Duration, SystemTime};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankRequest;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::value::RawValue;

use super::error::RerankCallError;
use super::transform::transform_request;
use crate::aws_endpoints::runtime::{BedrockEndpointType, RuntimeEndpointInput, runtime_endpoint};
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::signing::body::Body;
use crate::signing::endpoint::{RequestHeaders, request_headers};
use crate::signing::request::AwsSignedRequest;
use crate::signing::target::{RegionInputs, RequestTarget, TargetRequest, request_target};

pub type ParamMap = BTreeMap<String, Box<RawValue>>;

#[derive(Debug, Clone, Copy, Default)]
pub struct RerankEnvironment<'text> {
    pub region_name: Option<&'text str>,
    pub region: Option<&'text str>,
    pub session_region: Option<&'text str>,
    pub runtime_endpoint: Option<&'text str>,
}

pub struct RerankCall<'text> {
    pub request: &'text RerankRequest,
    pub optional_params: ParamMap,
    pub api_base: Option<&'text str>,
    pub extra_headers: Option<&'text [(String, String)]>,
    pub api_key: Option<&'text SecretString>,
    pub timeout: Option<Duration>,
    pub is_async: bool,
    pub environment: RerankEnvironment<'text>,
    pub now: SystemTime,
}

pub struct PreparedRequest {
    pub endpoint_url: String,
    pub signed: AwsSignedRequest,
    pub data: IndexMap<String, Json>,
}

pub struct LogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

pub trait CredentialSource {
    fn resolve(
        &self,
        settings: AwsSettings,
        region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send;
}

pub async fn resolve_target<Source>(
    call: &mut RerankCall<'_>,
    source: &Source,
) -> Result<RequestTarget, RerankCallError>
where
    Source: CredentialSource + Sync,
{
    // no bearer shortcut: LiteLLM prepares rerank with supports_bearer_token=False
    let target = request_target(
        TargetRequest {
            optional_params: &mut call.optional_params,
            model: Some(&call.request.model),
            region_inputs: RegionInputs {
                environment_region_name: call.environment.region_name,
                environment_region: call.environment.region,
                session_region: call.environment.session_region,
            },
            bearer_token: None,
        },
        |settings, region| source.resolve(settings.clone(), region.to_owned()),
    )
    .await?;
    Ok(target)
}

pub fn prepare_request(
    call: &RerankCall<'_>,
    target: &RequestTarget,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedRequest, RerankCallError> {
    let data = transform_request(call.request);
    let endpoint_url = rerank_endpoint(call, target);
    let body = dumps_object(&data).into_bytes();
    let signed = sign_payload(call, target, &endpoint_url, &body)?;
    log_pre_call(&data, &endpoint_url, &signed, log)?;
    Ok(PreparedRequest {
        endpoint_url,
        signed,
        data,
    })
}

fn rerank_endpoint(call: &RerankCall<'_>, target: &RequestTarget) -> String {
    let endpoint = runtime_endpoint(RuntimeEndpointInput {
        api_base: call.api_base,
        configured_endpoint: target.endpoint.as_deref(),
        environment_endpoint: call.environment.runtime_endpoint,
        region: &target.region,
        endpoint_type: BedrockEndpointType::Runtime,
    });
    // LiteLLM replaces every bedrock-runtime and appends /rerank without trimming slashes
    format!(
        "{}/rerank",
        endpoint
            .proxy_endpoint_url
            .replace("bedrock-runtime", "bedrock-agent-runtime")
    )
}

fn sign_payload(
    call: &RerankCall<'_>,
    target: &RequestTarget,
    endpoint_url: &str,
    body: &[u8],
) -> Result<AwsSignedRequest, RerankCallError> {
    let headers = content_headers(call.extra_headers);
    Ok(request_headers(RequestHeaders {
        credentials: target.credentials.as_ref(),
        region: &target.region,
        extra_headers: call.extra_headers.unwrap_or_default(),
        endpoint: endpoint_url,
        data: Body::Bytes(body),
        headers: &headers,
        api_key: call.api_key,
        configured_bearer: None,
        supports_bearer_token: false,
        now: call.now,
    })?)
}

fn content_headers(extra: Option<&[(String, String)]>) -> Vec<(String, Option<String>)> {
    let mut headers = vec![(
        "Content-Type".to_owned(),
        Some("application/json".to_owned()),
    )];
    for (name, value) in extra.unwrap_or_default() {
        match headers.iter_mut().find(|(existing, _)| existing == name) {
            Some((_, stored)) => *stored = Some(value.clone()),
            None => headers.push((name.clone(), Some(value.clone()))),
        }
    }
    headers
}

fn log_pre_call(
    data: &IndexMap<String, Json>,
    endpoint_url: &str,
    signed: &AwsSignedRequest,
    log: &mut LogContext<'_, '_>,
) -> Result<(), RerankCallError> {
    let mut headers = IndexMap::new();
    for (name, value) in &signed.headers {
        headers.insert(name.clone(), Json::Text(value.clone()));
    }
    pre_call(
        log.state,
        &PreCall {
            input: Json::Object(data.clone()),
            model: None,
            additional_args: AdditionalArgs {
                api_base: Some(endpoint_url.to_owned()),
                headers: Some(headers),
                complete_input_dict: Some(RawBody::Object(data.clone())),
                request_str: None,
                openai_sdk: None,
                passthrough: IndexMap::new(),
            },
        },
        log.env,
    )?;
    Ok(())
}
