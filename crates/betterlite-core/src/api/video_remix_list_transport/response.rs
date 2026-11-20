use betterlite_http::proxy::UpstreamResponse;

use crate::api::video_records::VideoObject;

use super::LogContext;
use super::config::VideoRemixListConfig;
use super::convert;
use super::error::{ConfigFailure, VideoRemixListTransportError};
use super::records::{Params, VideoLogging, VideoReply};

// A response transform, which is one of B3331's config trait members: remix and list each name their own
pub(super) trait ResponseTransform {
    type Output;

    fn run(
        config: &dyn VideoRemixListConfig,
        reply: VideoReply<'_>,
    ) -> Result<Self::Output, ConfigFailure>;
}

pub(super) struct RemixResponse;

impl ResponseTransform for RemixResponse {
    type Output = VideoObject;

    fn run(
        config: &dyn VideoRemixListConfig,
        reply: VideoReply<'_>,
    ) -> Result<Self::Output, ConfigFailure> {
        config.transform_video_remix_response(reply)
    }
}

pub(super) struct ListResponse;

impl ResponseTransform for ListResponse {
    type Output = Params;

    fn run(
        config: &dyn VideoRemixListConfig,
        reply: VideoReply<'_>,
    ) -> Result<Self::Output, ConfigFailure> {
        config.transform_video_list_response(reply)
    }
}

// The response transform runs against the real logging details, and whatever it stores there is kept
pub(super) fn transform_response<Kind>(
    config: &dyn VideoRemixListConfig,
    answer: &UpstreamResponse,
    provider: &str,
    log: &mut LogContext<'_, '_>,
) -> Result<Kind::Output, VideoRemixListTransportError>
where
    Kind: ResponseTransform,
{
    let mut logging = VideoLogging {
        model_call_details: log.state.model_call_details.clone(),
    };
    let outcome = Kind::run(
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
