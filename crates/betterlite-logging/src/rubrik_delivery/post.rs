use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use super::error::{RubrikError, Service};
use super::logger::RubrikLogger;
use super::transport::{ClientTier, JsonPost};

#[derive(Serialize)]
struct ResponseModerationEnvelope<'payload, Request, Response> {
    request: &'payload Request,
    response: &'payload Response,
}

pub async fn post_json<Payload>(
    logger: &RubrikLogger,
    endpoint: &str,
    payload: &Payload,
    service: Service,
) -> Result<IndexMap<String, Json>, RubrikError>
where
    Payload: Serialize + Sync,
{
    tracing::debug!(endpoint = %endpoint, %service, "Sending request to the Rubrik webhook");
    let body =
        serde_json::to_vec(payload).map_err(|source| RubrikError::Serialize { service, source })?;
    let request = JsonPost {
        tier: ClientTier::Moderation,
        url: endpoint.to_owned(),
        headers: logger.headers.clone(),
        body,
        timeout: logger.settings.moderation_timeout,
    };
    let response = logger
        .transport
        .post_json(request)
        .await
        .map_err(|source| RubrikError::Transport { service, source })?;
    if response.status >= 400 {
        return Err(RubrikError::HttpStatus {
            service,
            status: response.status,
        });
    }
    let parsed = serde_json::from_slice::<Json>(&response.body)
        .map_err(|source| RubrikError::ResponseBody { service, source })?;
    match parsed {
        Json::Object(fields) => Ok(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(RubrikError::NonObjectResponse { service })
        }
    }
}

pub async fn post_response_moderation<Request, Response>(
    logger: &RubrikLogger,
    response_data: &Response,
    request_data: &Request,
) -> Result<IndexMap<String, Json>, RubrikError>
where
    Request: Serialize + Sync,
    Response: Serialize + Sync,
{
    let envelope = ResponseModerationEnvelope {
        request: request_data,
        response: response_data,
    };
    post_json(
        logger,
        &logger.settings.response_moderation_endpoint,
        &envelope,
        Service::ResponseModeration,
    )
    .await
}

pub async fn post_prompt_moderation<Payload>(
    logger: &RubrikLogger,
    payload: &Payload,
) -> Result<IndexMap<String, Json>, RubrikError>
where
    Payload: Serialize + Sync,
{
    post_json(
        logger,
        &logger.settings.prompt_moderation_endpoint,
        payload,
        Service::PromptModeration,
    )
    .await
}
