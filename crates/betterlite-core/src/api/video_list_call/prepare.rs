use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use serde_json::Number;

use super::VideoListCall;
use crate::api::video_remix_call::config::{VideoOperation, resolved_timeout};
use crate::api::video_remix_call::error::VideoCallError;
use crate::api::video_remix_call::shared::{LoggedCall, call_kwargs, log_call, transport_params};
use crate::api::video_remix_list_transport::records::Params;

pub(super) struct PreparedList {
    pub(super) litellm_params: Params,
    pub(super) timeout: Duration,
}

// The steps between the provider config and the handler: GenericLiteLLMParams, the query params, the logging update, then `timeout or DEFAULT`
pub(super) fn prepare_list(
    logging: &mut LoggingState,
    call: &VideoListCall,
    provider: &str,
    request_timeout_seconds: f64,
) -> Result<PreparedList, VideoCallError> {
    let params = list_params(call);
    let litellm_params = transport_params(
        &call.extra,
        call.api_key.as_ref(),
        call.api_base.as_deref(),
        call.api_version.as_deref(),
    );
    let kwargs = call_kwargs(
        &call.extra,
        call.litellm_metadata.as_ref(),
        call.litellm_call_id.as_deref(),
    )?;
    log_call(
        logging,
        &LoggedCall {
            kwargs: &kwargs,
            optional_params: &params,
            call_id: call.litellm_call_id.as_deref(),
            provider,
            user: call.user.as_deref(),
            call_type: VideoOperation::List.call_type(),
        },
    )?;
    let timeout = resolved_timeout(call.timeout, request_timeout_seconds)?;
    Ok(PreparedList {
        litellm_params,
        timeout,
    })
}

// params = {"after": after, "limit": limit, "order": order}: a value the caller left out stays None
fn list_params(call: &VideoListCall) -> Params {
    let mut params = Params::new();
    params.insert(
        "after".to_owned(),
        call.after
            .as_ref()
            .map_or(Json::Null, |after| Json::Text(after.clone())),
    );
    params.insert(
        "limit".to_owned(),
        call.limit
            .map_or(Json::Null, |limit| Json::Number(Number::from(limit))),
    );
    params.insert(
        "order".to_owned(),
        call.order
            .as_ref()
            .map_or(Json::Null, |order| Json::Text(order.clone())),
    );
    params
}
