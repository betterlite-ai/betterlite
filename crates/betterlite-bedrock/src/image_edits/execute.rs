use std::str;

use betterlite_core::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use betterlite_openai::error::{ErrorRequest, ErrorResponse};

use super::error::ImageEditError;
use super::plan::{EditCall, PreparedRequest};
use crate::bedrock_settings::{
    BedrockError, ErrorHeaders, ErrorInput, ErrorReply, HeaderError, ResponseBody,
};
use crate::image_call::transport::{
    ClientFactory, ImageReply, ImageRequest, ImageTransport as _, TransferError,
};

// The transport is chosen only after signing, so both paths send the one prepared request.
pub async fn dispatch_request<Factory>(
    call: &EditCall<'_>,
    prepared: &PreparedRequest,
    supplied: Option<SuppliedClient<'_, Factory::Client>>,
    factory: &Factory,
) -> Result<ImageReply, ImageEditError>
where
    Factory: ClientFactory + Sync,
{
    let request = ImageRequest {
        url: prepared.endpoint_url.clone(),
        headers: prepared.signed.headers.clone(),
        body: prepared.signed.body.clone(),
    };
    let sent = match (call.aimage_edit, supplied) {
        (true, Some(client)) if client.kind == ClientKind::Asynchronous => {
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
        Err(error) => return Err(ImageEditError::Transport(error)),
    };
    if !(200..300).contains(&reply.status) {
        return Err(provider_error(&prepared.endpoint_url, &reply)?);
    }
    Ok(reply)
}

fn timeout_error() -> Result<ImageEditError, HeaderError> {
    let error = BedrockError::try_from(ErrorInput {
        status_code: 408,
        message: "the provider request timed out".to_owned(),
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: None,
        response: None,
    })?;
    Ok(ImageEditError::Provider {
        status: 408,
        error: Box::new(error),
    })
}

// LiteLLM reads response.text; betterlite refuses bytes that are not UTF-8 by naming their count.
pub(crate) fn response_text(body: &[u8]) -> String {
    str::from_utf8(body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", body.len()),
        str::to_owned,
    )
}

fn provider_error(endpoint: &str, reply: &ImageReply) -> Result<ImageEditError, HeaderError> {
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
    Ok(ImageEditError::Provider {
        status: reply.status,
        error: Box::new(error),
    })
}
