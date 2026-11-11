use std::str;

use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_models::decisions::Json;
use betterlite_openai::error::{ErrorRequest, ErrorResponse};
use indexmap::IndexMap;

use super::decode::decode_bedrock_image_reply;
use super::error::ImageCallError;
use super::plan::{ImageCall, LogContext, PreparedRequest};
use super::transform::apply_reply;
use super::transport::{
    ClientFactory, ClientKind, ImageReply, ImageRequest, ImageTransport as _, SuppliedClient,
    TransferError,
};
use crate::bedrock_settings::{
    BedrockError, ErrorHeaders, ErrorInput, ErrorReply, HeaderError, ResponseBody,
};
use crate::image_models::family::select_family;
use crate::image_models::response::ModelResponse;

// the transport is chosen only after signing, so both paths send one prepared request
pub async fn dispatch_request<Factory>(
    call: &ImageCall<'_>,
    prepared: &PreparedRequest,
    supplied: Option<SuppliedClient<'_, Factory::Client>>,
    factory: &Factory,
) -> Result<ImageReply, ImageCallError>
where
    Factory: ClientFactory + Sync,
{
    let request = ImageRequest {
        url: prepared.endpoint_url.clone(),
        headers: prepared.signed.headers.clone(),
        body: prepared.signed.body.clone(),
    };
    let sent = match (call.aimg_generation, supplied) {
        (true, Some(client)) if client.kind == ClientKind::Async => {
            client.transport.post(request).await
        }
        (true, _) => factory.async_client(call.timeout).post(request).await,
        (false, Some(client)) if client.kind == ClientKind::Sync => {
            client.transport.post(request).await
        }
        (false, _) => factory.sync_client().post(request).await,
    };
    let reply = match sent {
        Ok(reply) => reply,
        Err(TransferError::Timeout) => return Err(timeout_error()?),
        Err(error) => return Err(ImageCallError::Transport(error)),
    };
    if reply.status >= 400 {
        return Err(provider_error(&prepared.endpoint_url, &reply)?);
    }
    Ok(reply)
}

pub fn transform_response(
    call: &ImageCall<'_>,
    prepared: &PreparedRequest,
    reply: &ImageReply,
    mut model_response: ModelResponse,
    log: &mut LogContext<'_, '_>,
) -> Result<ModelResponse, ImageCallError> {
    log_post_call(call, prepared, reply, log);
    let family = select_family(Some(call.model));
    let record = decode_bedrock_image_reply(&reply.body, family)?;
    apply_reply(&mut model_response, record)?;
    Ok(model_response)
}

fn log_post_call(
    call: &ImageCall<'_>,
    prepared: &PreparedRequest,
    reply: &ImageReply,
    log: &mut LogContext<'_, '_>,
) {
    let text = response_text(&reply.body);
    post_call(
        log.state,
        &PostCall {
            original_response: Json::Text(text.clone()),
            input: Json::Text(call.prompt.to_owned()),
            additional_args: AdditionalArgs {
                api_base: None,
                headers: None,
                complete_input_dict: Some(RawBody::Object(prepared.data.clone())),
                request_str: None,
                openai_sdk: None,
                passthrough: IndexMap::new(),
            },
        },
        log.env,
    );
    tracing::debug!("raw model_response: {text}");
}

fn timeout_error() -> Result<ImageCallError, HeaderError> {
    let error = BedrockError::try_from(ErrorInput {
        status_code: 408,
        message: "the provider request timed out".to_owned(),
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: None,
        response: None,
    })?;
    Ok(ImageCallError::Provider {
        status: 408,
        error: Box::new(error),
    })
}

// LiteLLM reads response.text; betterlite refuses bytes that are not UTF-8 by naming their count
fn response_text(body: &[u8]) -> String {
    str::from_utf8(body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", body.len()),
        str::to_owned,
    )
}

fn provider_error(endpoint: &str, reply: &ImageReply) -> Result<ImageCallError, HeaderError> {
    let message = response_text(&reply.body);
    let request = ErrorRequest {
        method: "POST".to_owned(),
        url: endpoint.to_owned(),
    };
    let response = ErrorResponse {
        status_code: reply.status,
        request,
    };
    let error = BedrockError::try_from(ErrorInput {
        status_code: reply.status,
        message: message.clone(),
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: Some(ErrorHeaders::Http(reply.headers.clone())),
        response: Some(ErrorReply {
            response,
            headers: reply.headers.clone(),
            body: ResponseBody::Read(message),
        }),
    })?;
    Ok(ImageCallError::Provider {
        status: reply.status,
        error: Box::new(error),
    })
}
