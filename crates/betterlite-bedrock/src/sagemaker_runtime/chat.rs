use std::collections::VecDeque;

use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_models::decisions::Json as Plain;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::compatible_wire::{
    CompatibleReply, ReplyOptions, decode_compatible_chat_reply,
};
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::sagemaker_chat::nova;
use betterlite_openai::sagemaker_chat::request::{self as chat_request, TransformInput};
use betterlite_openai::sagemaker_completion::error::SagemakerError;
use betterlite_prompts::arguments::json::{Json, Object as Body};
use bytes::BytesMut;
use indexmap::IndexMap;

use crate::credentials::CredentialSource;
use crate::sagemaker_stream::events::EventState;
use crate::sagemaker_stream::{ErrorPolicy, SagemakerEvents};
use crate::signing::request::AwsSignedRequest;
use crate::signing::service::{ServiceName, ServiceRequest, sign_service_request};
use crate::signing::target::resolve_region;
use secrecy::SecretString;

use super::convert;
use super::endpoint::{self, EndpointRequest, ParamMap};
use super::error::{RuntimeError, timeout_error};
use super::runtime::{
    ChunkStream, PreparedCall, SagemakerRuntime, SagemakerTransport, body_text, reply_headers,
    stream_error,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatVariant {
    Chat,
    Nova,
}

pub struct ChatCall<'call> {
    pub model: &'call str,
    pub messages: &'call [ChatMessage],
    pub optional_params: &'call Body,
    pub signing: &'call ParamMap,
    pub headers: Headers,
    pub hf_model_name: Option<&'call str>,
    pub variant: ChatVariant,
    pub stream: bool,
    pub reply: ReplyOptions<'call>,
    // AWS_BEARER_TOKEN_BEDROCK as B682 resolved it; the handler's sign_request never forwards an api_key, so only this one reaches _sign_request
    pub configured_bearer: Option<&'call SecretString>,
}

pub async fn prepare_chat<T, C>(
    runtime: &SagemakerRuntime<'_, T, C>,
    call: &ChatCall<'_>,
) -> Result<PreparedCall, RuntimeError>
where
    T: Sync,
    C: CredentialSource,
{
    let base_url = text_field(call.optional_params, "sagemaker_base_url")?;
    let region = resolve_region(call.signing, Some(call.model), runtime.environment.inputs())?;
    let url = endpoint::runtime_url(&EndpointRequest {
        model: call.model,
        region: &region,
        base_url,
        stream: call.stream,
    });
    let body_json = chat_body_json(call)?;
    let signed = sign_chat(runtime, call, &url, &body_json).await?;
    Ok(PreparedCall {
        url: signed.url,
        headers: signed.headers,
        body: signed.body,
        body_json,
        region,
    })
}

async fn sign_chat<T, C>(
    runtime: &SagemakerRuntime<'_, T, C>,
    call: &ChatCall<'_>,
    url: &str,
    body_json: &IndexMap<String, Plain>,
) -> Result<AwsSignedRequest, RuntimeError>
where
    T: Sync,
    C: CredentialSource,
{
    let headers = chat_request::validate_environment(call.headers.clone(), call.optional_params);
    let pairs: Vec<(String, Option<String>)> = headers
        .into_iter()
        .map(|(name, value)| (name, Some(value)))
        .collect();
    sign_service_request(
        ServiceRequest {
            service: ServiceName::Sagemaker,
            headers: &pairs,
            optional_params: call.signing,
            request_data: body_json,
            api_base: url,
            model: Some(call.model),
            api_key: None,
            configured_bearer: call.configured_bearer,
            region_inputs: runtime.environment.inputs(),
            now: runtime.clock.now(),
        },
        |settings, region| {
            runtime
                .credentials
                .resolve(settings.clone(), region.to_owned())
        },
    )
    .await
    .map_err(RuntimeError::from)
}

fn chat_body_json(call: &ChatCall<'_>) -> Result<IndexMap<String, Plain>, RuntimeError> {
    let input = TransformInput {
        model: call.model,
        messages: call.messages,
        optional_params: call.optional_params,
        hf_model_name: call.hf_model_name,
    };
    let body = match call.variant {
        ChatVariant::Chat => chat_request::transform_request(&input)?,
        ChatVariant::Nova => nova::transform_request(&input)?,
    };
    Ok(convert::prompt_body(&body))
}

pub async fn chat<T, C>(
    runtime: &SagemakerRuntime<'_, T, C>,
    call: &ChatCall<'_>,
) -> Result<CompatibleReply, SagemakerError>
where
    T: SagemakerTransport,
    C: CredentialSource,
{
    let response = runtime
        .transport
        .send(runtime.outbound(&prepare_chat(runtime, call).await?)?)
        .await
        .map_err(timeout_error)?;
    // a redirect without a Location reaches here as a wire error
    if !(200..300).contains(&response.status) {
        return Err(SagemakerError {
            status_code: response.status,
            message: body_text(&response.body),
            headers: Some(reply_headers(&response.headers)),
        });
    }
    decode_compatible_chat_reply(&response.body, &call.reply).map_err(|error| SagemakerError {
        status_code: response.status,
        message: error.to_string(),
        headers: Some(reply_headers(&response.headers)),
    })
}

pub async fn chat_stream<T, C>(
    runtime: &SagemakerRuntime<'_, T, C>,
    call: &ChatCall<'_>,
) -> Result<SagemakerEvents<ChunkStream>, SagemakerError>
where
    T: SagemakerTransport,
    C: CredentialSource,
{
    open_chat_stream(
        &runtime.transport,
        runtime.outbound(&prepare_chat(runtime, call).await?)?,
    )
    .await
}

async fn open_chat_stream<T>(
    transport: &T,
    request: OutboundRequest,
) -> Result<SagemakerEvents<ChunkStream>, SagemakerError>
where
    T: SagemakerTransport,
{
    let (status, chunks) = transport
        .open_stream(request)
        .await
        .map_err(timeout_error)?;
    if status != 200 {
        return Err(stream_error(status, chunks).await);
    }
    Ok(SagemakerEvents {
        state: EventState {
            // the chat stream wrapper builds AWSEventStreamDecoder(model="", is_messages_api=True)
            model: String::new(),
            is_messages_api: true,
            policy: ErrorPolicy::Skip,
            frames: BytesMut::new(),
            accumulated: String::new(),
        },
        chunks,
        pending: VecDeque::new(),
        done: false,
    })
}

fn text_field<'body>(
    params: &'body Body,
    name: &'static str,
) -> Result<Option<&'body str>, RuntimeError> {
    match params.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(_other) => Err(RuntimeError::Field { field: name }),
    }
}

mod tests;
