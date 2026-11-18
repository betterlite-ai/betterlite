use std::collections::VecDeque;

use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::ChatContentError;
use betterlite_openai::compatible_wire::{
    CompatibleReply, ReplyOptions, decode_compatible_chat_reply,
};
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::sagemaker_chat::nova;
use betterlite_openai::sagemaker_chat::request::{self as chat_request, TransformInput};
use betterlite_openai::sagemaker_completion::error::SagemakerError;
use betterlite_prompts::arguments::json::{Json, Object as Body};
use bytes::BytesMut;
use http::HeaderMap;

use crate::sagemaker_stream::events::EventState;
use crate::sagemaker_stream::{ErrorPolicy, SagemakerEvents};
use crate::signing::service::{ServiceName, ServiceRequest, sign_service_request};
use crate::signing::target::resolve_region;

use super::convert;
use super::endpoint::{self, EndpointRequest, ParamMap};
use super::error::RuntimeError;
use super::runtime::{
    ChunkStream, PreparedCall, SagemakerCredentials, SagemakerRuntime, SagemakerTransport,
    body_text, reply_headers, stream_error,
};

const BASE_URL_FIELD: &str = "sagemaker_base_url";

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
}

impl<T, C> SagemakerRuntime<'_, T, C>
where
    C: SagemakerCredentials,
{
    // get_complete_url, validate_environment, transform_request and _sign_request, in the handler's order
    pub async fn prepare_chat(&self, call: &ChatCall<'_>) -> Result<PreparedCall, RuntimeError> {
        let base_url = text_field(call.optional_params, BASE_URL_FIELD)?;
        let region = resolve_region(call.signing, Some(call.model), self.environment.inputs())?;
        let url = endpoint::runtime_url(&EndpointRequest {
            model: call.model,
            region: &region,
            base_url,
            stream: call.stream,
        });
        let input = TransformInput {
            model: call.model,
            messages: call.messages,
            optional_params: call.optional_params,
            hf_model_name: call.hf_model_name,
        };
        let body = request_body(call.variant, &input)?;
        let body_json = convert::prompt_body(&body);
        let headers =
            chat_request::validate_environment(call.headers.clone(), call.optional_params);
        let pairs: Vec<(String, Option<String>)> = headers
            .into_iter()
            .map(|(name, value)| (name, Some(value)))
            .collect();
        let signed = sign_service_request(
            ServiceRequest {
                service: ServiceName::Sagemaker,
                headers: &pairs,
                optional_params: call.signing,
                request_data: &body_json,
                api_base: &url,
                model: Some(call.model),
                api_key: None,
                configured_bearer: None,
                region_inputs: self.environment.inputs(),
                now: self.clock.now(),
            },
            |settings, region| {
                self.credentials
                    .resolve(settings.clone(), region.to_owned())
            },
        )
        .await?;
        Ok(PreparedCall {
            url: signed.url,
            headers: signed.headers,
            body: signed.body,
            body_json,
            region,
        })
    }
}

impl<T, C> SagemakerRuntime<'_, T, C>
where
    T: SagemakerTransport,
    C: SagemakerCredentials,
{
    pub async fn chat(&self, call: &ChatCall<'_>) -> Result<CompatibleReply, SagemakerError> {
        let request = self.chat_request(call).await?;
        let response = self.transport.send(request).await?;
        read_chat_response(
            response.status,
            &response.body,
            &response.headers,
            &call.reply,
        )
    }

    pub async fn chat_stream(
        &self,
        call: &ChatCall<'_>,
    ) -> Result<SagemakerEvents<ChunkStream>, SagemakerError> {
        let request = self.chat_request(call).await?;
        let (status, chunks) = self.transport.open_stream(request).await?;
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
}

impl<T, C> SagemakerRuntime<'_, T, C>
where
    C: SagemakerCredentials,
{
    async fn chat_request(&self, call: &ChatCall<'_>) -> Result<OutboundRequest, RuntimeError> {
        let prepared = self.prepare_chat(call).await?;
        let request = self.outbound(&prepared)?;
        Ok(request)
    }
}

// SagemakerNovaConfig.transform_request builds the chat body and drops the model field
fn request_body(
    variant: ChatVariant,
    input: &TransformInput<'_>,
) -> Result<Body, ChatContentError> {
    match variant {
        ChatVariant::Chat => chat_request::transform_request(input),
        ChatVariant::Nova => nova::transform_request(input),
    }
}

fn read_chat_response(
    status: u16,
    body: &[u8],
    headers: &HeaderMap,
    options: &ReplyOptions<'_>,
) -> Result<CompatibleReply, SagemakerError> {
    // only a 2xx reply is the answer; a redirect without a Location reaches here as a wire error
    if !(200..300).contains(&status) {
        return Err(SagemakerError {
            status_code: status,
            message: body_text(body),
            headers: Some(reply_headers(headers)),
        });
    }
    decode_compatible_chat_reply(body, options).map_err(|error| SagemakerError {
        status_code: status,
        message: error.to_string(),
        headers: Some(reply_headers(headers)),
    })
}

// a null entry is the absent one, as dict.get reads it; any other non-string names the field
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
