use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;

use super::VideoRemixCall;
use super::config::{VideoOperation, resolved_timeout};
use super::error::VideoCallError;
use super::shared::{LoggedCall, call_kwargs, log_call, transport_params};
use crate::api::video_remix_list_transport::records::Params;

pub(super) struct PreparedRemix {
    pub(super) litellm_params: Params,
    pub(super) timeout: Duration,
}

// The steps between the provider config and the handler: GenericLiteLLMParams, the request params, the logging update, then `timeout or DEFAULT`
pub(super) fn prepare_remix(
    logging: &mut LoggingState,
    call: &VideoRemixCall,
    provider: &str,
    request_timeout_seconds: f64,
) -> Result<PreparedRemix, VideoCallError> {
    let params = remix_params(call);
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
            call_type: VideoOperation::Remix.call_type(),
        },
    )?;
    let timeout = resolved_timeout(call.timeout, request_timeout_seconds)?;
    Ok(PreparedRemix {
        litellm_params,
        timeout,
    })
}

// params = {"video_id": video_id, "prompt": prompt}
fn remix_params(call: &VideoRemixCall) -> Params {
    let mut params = Params::new();
    params.insert("video_id".to_owned(), Json::Text(call.video_id.clone()));
    params.insert("prompt".to_owned(), Json::Text(call.prompt.clone()));
    params
}
