use std::future::Future;
use std::time::SystemTime;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;
use uuid::Uuid;

use super::error::InvokeAgentRequestError;
use super::payload::transform_request;
use super::signing::{SignRequest, sign_request};
use super::url::{CompleteUrl, InvokeEnvironment, complete_url};
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::model_routes::ParamMap;
use crate::signing::request::AwsSignedRequest;

pub struct PrepareRequest<'text> {
    pub model: &'text str,
    pub messages: &'text [Message],
    pub optional_params: &'text ParamMap,
    pub headers: &'text [(String, Option<String>)],
    pub api_base: Option<&'text str>,
    pub environment: InvokeEnvironment<'text>,
    pub api_key: Option<&'text SecretString>,
    pub configured_bearer: Option<&'text SecretString>,
    pub stream: Option<bool>,
    pub fake_stream: Option<bool>,
    pub now: SystemTime,
    pub generated_session_id: Uuid,
}

pub async fn prepare_request<F, Fut>(
    request: PrepareRequest<'_>,
    resolve_credentials: F,
) -> Result<AwsSignedRequest, InvokeAgentRequestError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let url = complete_url(CompleteUrl {
        api_base: request.api_base,
        model: request.model,
        optional_params: request.optional_params,
        environment: request.environment,
        generated_session_id: request.generated_session_id,
    })?;
    let request_data = transform_request(request.messages, request.optional_params)?;
    sign_request(
        SignRequest {
            headers: request.headers,
            optional_params: request.optional_params,
            request_data: &request_data,
            api_base: &url,
            model: Some(request.model),
            api_key: request.api_key,
            configured_bearer: request.configured_bearer,
            stream: request.stream,
            fake_stream: request.fake_stream,
            region_inputs: request.environment.region_inputs,
            now: request.now,
        },
        resolve_credentials,
    )
    .await
}
