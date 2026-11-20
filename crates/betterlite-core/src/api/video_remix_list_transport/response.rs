use betterlite_http::proxy::UpstreamResponse;

use super::LogContext;
use super::config::VideoRemixListConfig;
use super::convert;
use super::error::{ConfigFailure, VideoRemixListTransportError};
use super::records::{VideoLogging, VideoReply};

// A response transform, which is one of B3331's config trait members: remix and list each name their own
pub(super) type ResponseTransform<T> =
    fn(&dyn VideoRemixListConfig, VideoReply<'_>) -> Result<T, ConfigFailure>;

// The response transform runs against the real logging details, and whatever it stores there is kept
pub(super) fn transform_response<T>(
    config: &dyn VideoRemixListConfig,
    answer: &UpstreamResponse,
    provider: &str,
    log: &mut LogContext<'_, '_>,
    transform: ResponseTransform<T>,
) -> Result<T, VideoRemixListTransportError> {
    let mut logging = VideoLogging {
        model_call_details: log.state.model_call_details.clone(),
    };
    let outcome = transform(
        config,
        VideoReply {
            raw_response: answer,
            logging_obj: &mut logging,
            custom_llm_provider: Some(provider),
        },
    );
    log.state.model_call_details = logging.model_call_details;
    outcome.map_err(|_failure| convert::failure(config, &convert::status_facts(answer)))
}
