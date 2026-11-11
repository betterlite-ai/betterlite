use std::future::Future;
use std::time::SystemTime;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use secrecy::SecretString;
use uuid::Uuid;

use super::error::AgentCoreRequestError;
use super::message::AgentCoreMessage;
use super::payload::transform_request;
use super::signing::{SignRequest, sign_request};
use super::url::complete_url;
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::model_routes::ParamMap;
use crate::signing::request::AwsSignedRequest;
use crate::signing::target::RegionInputs;

pub struct PrepareRequest<'text> {
    pub model: &'text str,
    pub messages: &'text [AgentCoreMessage],
    pub optional_params: &'text ParamMap,
    pub litellm_params: &'text ParamMap,
    pub headers: &'text [(String, Option<String>)],
    pub api_key: Option<&'text SecretString>,
    pub configured_bearer: Option<&'text SecretString>,
    pub region_inputs: RegionInputs<'text>,
    pub now: SystemTime,
    pub generated_session_id: Uuid,
}

pub async fn prepare_request<F, Fut>(
    request: PrepareRequest<'_>,
    resolve_credentials: F,
) -> Result<AwsSignedRequest, AgentCoreRequestError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let url = complete_url(request.model, request.optional_params)?;
    let mut headers = request.headers.to_vec();
    let request_data = transform_request(
        request.messages,
        request.optional_params,
        request.litellm_params,
        &mut headers,
        request.generated_session_id,
    )?;
    sign_request(
        SignRequest {
            headers: &headers,
            optional_params: request.optional_params,
            request_data: &request_data,
            api_base: &url,
            model: Some(request.model),
            api_key: request.api_key,
            configured_bearer: request.configured_bearer,
            region_inputs: request.region_inputs,
            now: request.now,
        },
        resolve_credentials,
    )
    .await
}
