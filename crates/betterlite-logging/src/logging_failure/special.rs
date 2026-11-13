use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::logging_failure::{
    FailureReport, NO_DEPLOYMENTS_AVAILABLE, RateLimitFailureEvent, RateLimitFailureListener,
};
use crate::logging_state::LoggingState;

pub async fn special_failure_handlers(
    state: &LoggingState,
    report: &FailureReport,
    listeners: &[Arc<dyn RateLimitFailureListener>],
) {
    let metadata = metadata_of(state);
    if !metadata.is_some_and(is_single_deployment)
        && !report.error_text().contains(NO_DEPLOYMENTS_AVAILABLE)
    {
        return;
    }
    let original_model_group = metadata.and_then(model_group);
    for listener in listeners {
        let event = RateLimitFailureEvent {
            exception: &report.exception,
            original_model_group,
            details: &state.model_call_details,
        };
        if let Err(error) = listener.log_model_group_rate_limit_error(&event).await {
            tracing::warn!(
                %error,
                callback = listener.callback_name(),
                "the model group rate limit listener errored"
            );
        }
    }
}

fn metadata_of(state: &LoggingState) -> Option<&IndexMap<String, Json>> {
    match state.litellm_params.get("metadata")? {
        Json::Object(metadata) => Some(metadata),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn is_single_deployment(metadata: &IndexMap<String, Json>) -> bool {
    let Some(Json::Number(size)) = metadata.get("model_group_size") else {
        return false;
    };
    size.as_i64() == Some(1)
}

fn model_group(metadata: &IndexMap<String, Json>) -> Option<&str> {
    let value = metadata.get("model_group")?;
    match value {
        Json::Text(group) => Some(group),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}
