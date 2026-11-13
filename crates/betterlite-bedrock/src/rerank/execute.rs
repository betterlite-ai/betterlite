use std::str;

use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_openai::error::{ErrorRequest, ErrorResponse};

use super::decode::decode_rerank_reply;
use super::error::RerankCallError;
use super::plan::{CredentialSource, LogContext, PreparedRequest, RerankCall, prepare_request};
use super::transport::{
    ClientFactory, ClientKind, RerankHttpReply, RerankHttpRequest, SuppliedClient, TransferError,
};
use crate::bedrock_settings::{
    BedrockError, ErrorHeaders, ErrorInput, ErrorReply, HeaderError, ResponseBody,
};
use crate::signing::target::{RegionInputs, TargetRequest, request_target};

pub async fn resolve_and_prepare<Source>(
    call: &mut RerankCall<'_>,
    source: &Source,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedRequest, RerankCallError>
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
    prepare_request(call, &target, log)
}

// the transport is chosen only after signing, so both paths send the one prepared request
pub async fn dispatch_request<Factory>(
    call: &RerankCall<'_>,
    prepared: &PreparedRequest,
    supplied: Option<SuppliedClient<'_, Factory::Client>>,
    factory: &Factory,
    log: &mut LogContext<'_, '_>,
) -> Result<RerankHttpReply, RerankCallError>
where
    Factory: ClientFactory + Sync,
{
    let request = RerankHttpRequest {
        url: prepared.endpoint_url.clone(),
        headers: prepared.signed.headers.clone(),
        body: prepared.signed.body.clone(),
        timeout: call.timeout,
    };
    let sent = match (call.is_async, supplied) {
        (true, Some(client)) if client.kind == ClientKind::Async => {
            client.transport.post_with_logging(request, log.state).await
        }
        (true, _) => {
            factory
                .async_client()
                .post_with_logging(request, log.state)
                .await
        }
        (false, Some(client)) if client.kind == ClientKind::Sync => {
            client.transport.post(request).await
        }
        (false, _) => factory.sync_client().post(request).await,
    };
    let reply = match sent {
        Ok(reply) => reply,
        Err(TransferError::Timeout) => return Err(timeout_error()?),
        Err(error) => return Err(RerankCallError::Transport(error)),
    };
    // LiteLLM calls raise_for_status, which raises only for 4xx and 5xx
    if (400..=599).contains(&reply.status) {
        return Err(provider_error(&prepared.endpoint_url, &reply)?);
    }
    Ok(reply)
}

pub fn transform_response(
    call: &RerankCall<'_>,
    reply: &RerankHttpReply,
    generated_id: &str,
    log: &mut LogContext<'_, '_>,
) -> Result<RerankResponse, RerankCallError> {
    // LiteLLM logs the raw text before it decodes the JSON, and only on the sync path
    if !call.is_async {
        post_call(
            log.state,
            &PostCall {
                original_response: Json::Text(response_text(&reply.body)),
                input: Json::Null,
                additional_args: AdditionalArgs::default(),
            },
            log.env,
        );
    }
    Ok(decode_rerank_reply(&reply.body, generated_id)?)
}

fn timeout_error() -> Result<RerankCallError, HeaderError> {
    let error = BedrockError::try_from(ErrorInput {
        status_code: 408,
        message: "Timeout error occurred.".to_owned(),
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: None,
        response: None,
    })?;
    Ok(RerankCallError::Provider {
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

fn provider_error(endpoint: &str, reply: &RerankHttpReply) -> Result<RerankCallError, HeaderError> {
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
    Ok(RerankCallError::Provider {
        status: reply.status,
        error: Box::new(error),
    })
}
