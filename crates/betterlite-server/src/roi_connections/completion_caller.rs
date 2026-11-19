use std::sync::Arc;

use axum::http::Method;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_roi::estimator::models::{
    ROICompletionMessage, ROICompletionMetadata, ROICompletionRequest, ROIResponseFormat,
    ReasoningEffort,
};
use betterlite_roi::settings::ROISettings;
use secrecy::SecretString;
use serde::Serialize;
use serde_json::value::RawValue;

use crate::roi_estimator::{CompletionCaller, CompletionFuture};
use crate::roi_github::error::SourceError;
use crate::roi_github::transport::Transport;

use super::error::RoiConnectionsError;
use super::gateway_http_client::{GatewayError, GatewayHttpClient};
use super::gateway_key::gateway_key;
use super::state::RoiConnectionsState;

const COMPLETIONS_PATH: &str = "/v1/chat/completions";
const GATEWAY_LABEL: &str = "The estimator gateway returned an error.";
const REQUEST_FAILED: &str = "The estimator completion request could not be written.";
const RESPONSE_INVALID: &str = "The estimator gateway returned a response that is not valid JSON.";

#[derive(Serialize)]
struct CompletionBody<'request> {
    model: &'request str,
    temperature: u32,
    messages: &'request [ROICompletionMessage],
    response_format: &'request ROIResponseFormat,
    max_tokens: u32,
    metadata: &'request ROICompletionMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'request ReasoningEffort>,
}

pub fn completion_caller<T>(
    settings: &ROISettings,
    state: &RoiConnectionsState<T>,
) -> Result<CompletionCaller, RoiConnectionsError>
where
    T: Transport + Clone + 'static,
{
    let credential = gateway_key(settings, state.master_key.as_ref())?;
    let client = GatewayHttpClient {
        base: state.gateway_base.clone(),
        transport: state.http.clone(),
    };
    let caller = move |request: ROICompletionRequest| -> CompletionFuture {
        let gateway_client = client.clone();
        let bearer = credential.clone();
        Box::pin(async move { post_completion(&gateway_client, &bearer, &request).await })
    };
    Ok(Arc::new(caller))
}

async fn post_completion<T>(
    client: &GatewayHttpClient<T>,
    credential: &SecretString,
    request: &ROICompletionRequest,
) -> Result<Box<RawValue>, SourceError>
where
    T: Transport,
{
    let body = completion_body(request)?;
    let response = match client
        .request(
            Method::POST,
            COMPLETIONS_PATH,
            credential,
            RequestBody::Json(body),
        )
        .await
    {
        Ok(response) => response,
        Err(GatewayError::Status { status }) => {
            return Err(SourceError::Http {
                label: GATEWAY_LABEL,
                status,
            });
        }
        Err(GatewayError::Transport(source)) => return Err(SourceError::Transport(source)),
        Err(GatewayError::Header(source)) => return Err(SourceError::Token(source)),
        Err(GatewayError::Url(source)) => return Err(SourceError::Url(source)),
    };
    serde_json::from_slice::<Box<RawValue>>(&response.body).map_err(|source| {
        SourceError::Pagination {
            message: RESPONSE_INVALID.to_owned(),
            source,
        }
    })
}

fn completion_body(request: &ROICompletionRequest) -> Result<Vec<u8>, SourceError> {
    let view = CompletionBody {
        model: &request.model,
        temperature: request.temperature,
        messages: &request.messages,
        response_format: &request.response_format,
        max_tokens: request.max_tokens,
        metadata: &request.metadata,
        reasoning_effort: request.reasoning_effort.as_ref(),
    };
    // the callback error type carries JSON failures in its one message-and-source variant
    serde_json::to_vec(&view).map_err(|source| SourceError::Pagination {
        message: REQUEST_FAILED.to_owned(),
        source,
    })
}

mod tests;
