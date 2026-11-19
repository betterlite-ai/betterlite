use std::future::Future;
use std::time::SystemTime;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::aws_settings::sources::Credentials;
use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput, HeaderError};
use crate::credentials::error::CredentialError;
use crate::invoke_primitives::config::Params;
use crate::invoke_primitives::environment::EnvironmentError;
use crate::invoke_primitives::signing::{SignRequest, sign_request};
use crate::invoke_primitives::url::{
    CompleteUrl, InvokeEnvironment, InvokeModelUrl, UrlError, invoke_model_url,
};
use crate::request_policy::{BedrockRequestPolicy, Params as MetadataParams, merge_invoke_headers};
use crate::signing::error::SigningError;
use crate::signing::request::AwsSignedRequest;
use crate::signing::target::RegionInputs;

pub struct MessagesEndpoint<'request> {
    pub api_base: Option<&'request str>,
    pub model: &'request str,
    pub optional_params: &'request Params,
    // LiteLLM types this bool | None and streams only for the literal true
    pub stream: Option<bool>,
    pub environment: InvokeEnvironment<'request>,
}

// AmazonInvokeConfig.get_complete_url behind the messages config: the provider picks the invoke-with-response-stream suffix and the endpoint keeps its proxy and base distinctions.
pub fn messages_complete_url(request: &MessagesEndpoint<'_>) -> Result<InvokeModelUrl, UrlError> {
    invoke_model_url(CompleteUrl {
        api_base: request.api_base,
        model: request.model,
        optional_params: request.optional_params,
        stream: request.stream == Some(true),
        environment: request.environment,
    })
}

pub struct MessagesSignRequest<'request> {
    pub headers: &'request IndexMap<String, String>,
    pub optional_params: &'request Params,
    pub request_data: &'request IndexMap<String, Json>,
    pub api_base: &'request str,
    pub api_key: Option<&'request SecretString>,
    pub configured_bearer: Option<&'request SecretString>,
    pub model: Option<&'request str>,
    pub stream: Option<bool>,
    pub fake_stream: Option<bool>,
    pub region_inputs: RegionInputs<'request>,
    pub now: SystemTime,
}

// AmazonInvokeConfig.sign_request behind the messages config; the signer reads neither stream flag, so both travel exactly as the caller supplied them.
pub async fn sign_messages_request<Resolve, Fut>(
    request: MessagesSignRequest<'_>,
    resolve_credentials: Resolve,
) -> Result<AwsSignedRequest, SigningError>
where
    Resolve: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let headers: Vec<(String, Option<String>)> = request
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), Some(value.clone())))
        .collect();
    sign_request(
        SignRequest {
            headers: &headers,
            optional_params: request.optional_params,
            request_data: request.request_data,
            api_base: request.api_base,
            api_key: request.api_key,
            configured_bearer: request.configured_bearer,
            model: request.model,
            stream: request.stream,
            fake_stream: request.fake_stream,
            region_inputs: request.region_inputs,
            now: request.now,
        },
        resolve_credentials,
    )
    .await
}

// get_error_class hands the caller's status, message and headers to LiteLLM's BedrockError unchanged; it fabricates no type, param or code.
pub fn get_error_class(
    message: &str,
    status_code: u16,
    headers: &[(String, String)],
) -> Result<BedrockError, HeaderError> {
    BedrockError::try_from(ErrorInput {
        status_code,
        message: message.to_owned(),
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: Some(ErrorHeaders::Http(headers.to_vec())),
        response: None,
    })
}

// LiteLLM's messages environment validation merges only the metadata header: no guardrail headers, the api_base it was given, and api_key never becomes a header.
pub fn validate_messages_environment(
    headers: &IndexMap<String, String>,
    litellm_params: &MetadataParams,
    policy: Option<&BedrockRequestPolicy>,
    api_base: Option<&str>,
    _api_key: Option<&SecretString>,
) -> Result<(IndexMap<String, String>, Option<String>), EnvironmentError> {
    let merged = merge_invoke_headers(headers, &[], policy, litellm_params)?;
    let unchanged = api_base.map(str::to_owned);
    Ok((merged, unchanged))
}
