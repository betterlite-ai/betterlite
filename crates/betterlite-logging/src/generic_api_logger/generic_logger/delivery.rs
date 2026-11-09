use super::super::generic_settings::LogFormat;
use super::super::v1_payload::{V1Kwargs, v1_logging_payload};
use super::GenericApiLogger;
use super::error::ApiError;
use super::retry::post_with_retries;
use crate::fields::payload::StandardLoggingPayload;
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};
use betterlite_models::decisions::Json;
use futures_util::future::join_all;
mod tests;
pub(super) fn standard_entry(payload: &StandardLoggingPayload) -> Result<Json, ApiError> {
    let value = serde_json::to_value(payload).map_err(ApiError::Payload)?;
    let entry = serde_json::from_value(value).map_err(ApiError::Payload)?;
    Ok(entry)
}
pub(super) fn v1_entry(
    payload: &StandardLoggingPayload,
    generated_id: &str,
) -> Result<Json, ApiError> {
    let kwargs = V1Kwargs {
        call_type: Some(payload.call_type.clone()),
        cache_hit: payload.cache_hit,
        model: Some(payload.model.clone()),
        user: payload.end_user.clone(),
        model_parameters: Some(payload.model_parameters.clone()),
        messages: payload.messages.clone(),
        metadata: Some(
            serde_json::from_value(
                serde_json::to_value(&payload.metadata).map_err(ApiError::Payload)?,
            )
            .map_err(ApiError::Payload)?,
        ),
        cost: Some(payload.response_cost),
    };
    let response = payload.response.as_ref().unwrap_or(&Json::Null);
    let entries = v1_logging_payload(
        &kwargs,
        response,
        payload.start_time,
        payload.end_time,
        generated_id,
    )?;
    Ok(Json::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key, Json::Text(value)))
            .collect(),
    ))
}
async fn deliver_single(logger: &GenericApiLogger, batch: &[Json]) {
    let posts = batch.iter().map(|entry| async move {
        let body = safe_dumps(entry, DEFAULT_MAX_RECURSE_DEPTH, None)?;
        post_with_retries(logger, &body).await
    });
    for (index, result) in join_all(posts).await.into_iter().enumerate() {
        if let Err(error) = result {
            tracing::error!("Generic API Logger - Error sending log {index}: {error}");
        }
    }
}

pub(super) async fn deliver(logger: &GenericApiLogger, batch: &[Json]) -> Result<(), ApiError> {
    match logger.settings.log_format {
        LogFormat::JsonArray => {
            let body = safe_dumps(&batch, DEFAULT_MAX_RECURSE_DEPTH, None)?;
            post_with_retries(logger, &body).await
        }
        LogFormat::Ndjson => {
            let entries: Result<Vec<String>, ApiError> = batch
                .iter()
                .map(|entry| {
                    safe_dumps(entry, DEFAULT_MAX_RECURSE_DEPTH, None).map_err(ApiError::Dump)
                })
                .collect();
            post_with_retries(logger, &entries?.join("\n")).await
        }
        LogFormat::Single => {
            deliver_single(logger, batch).await;
            Ok(())
        }
    }
}
