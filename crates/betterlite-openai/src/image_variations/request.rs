use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;

use super::call::{PreparedImageVariation, prepare_image_variation};
use super::error::{ImageVariationResult, ImageVariationSource, OpenAiImageVariationError};
use super::logging::{ImageVariationLogger, PostCallRecord, PreCallRecord};
use super::transform::usage_identity;
use super::transport::ImageVariationTransport;
use super::wire::outbound_request;
use crate::api::image_contracts::records::ImageHeaders;
use crate::image_generation::decode::{decode_image_reply, original_response};
use crate::image_generation::reply::{ImageGenerationReply, ImageReplyError};

#[derive(Debug)]
pub struct ImageVariationOutcome {
    pub reply: ImageGenerationReply,
    pub original_response: Json,
}

pub fn begin_image_variation(
    request: super::ImageVariationRequest,
    logger: &mut dyn ImageVariationLogger,
) -> ImageVariationResult<PreparedImageVariation> {
    let prepared = prepare_image_variation(request)?;
    let record = PreCallRecord {
        headers: prepared.headers.clone(),
        api_base: prepared.api_base.clone(),
        data: prepared.data.clone(),
    };
    if let Err(source) = logger.pre_call(record) {
        return Err(OpenAiImageVariationError::wrapped(
            ImageVariationSource::Logging(source),
        ));
    }
    Ok(prepared)
}

pub async fn send_image_variation<T>(
    prepared: &PreparedImageVariation,
    transport: &T,
) -> Result<ImageVariationOutcome, ImageVariationSource>
where
    T: ImageVariationTransport,
{
    let response = transport
        .create_variation(outbound_request(prepared)?)
        .await
        .map_err(ImageVariationSource::Transport)?;
    checked_reply(&response, prepared.now_seconds)
}

// the handler's post_call runs on a parsed reply only; a failed call reaches the caller without one
pub fn finish_image_variation(
    prepared: &PreparedImageVariation,
    sent: Result<ImageVariationOutcome, ImageVariationSource>,
    logger: &mut dyn ImageVariationLogger,
) -> ImageVariationResult<ImageVariationOutcome> {
    let outcome = match sent {
        Ok(outcome) => outcome,
        Err(source) => return Err(OpenAiImageVariationError::wrapped(source)),
    };
    logger.post_call(PostCallRecord {
        headers: prepared.headers.clone(),
        api_base: prepared.api_base.clone(),
        original_response: outcome.original_response.clone(),
    });
    Ok(outcome)
}

fn checked_reply(
    response: &UpstreamResponse,
    created_seconds: i64,
) -> Result<ImageVariationOutcome, ImageVariationSource> {
    if !(200..=299).contains(&response.status) {
        // httpx decodes an error body with replacement characters, so a non-UTF-8 one still keeps its status
        let (text, _, _) = encoding_rs::UTF_8.decode(&response.body);
        return Err(ImageVariationSource::Status {
            status: response.status,
            message: text.into_owned(),
            headers: Some(response_headers(&response.headers)?),
        });
    }
    let original = original_response(&response.body)?;
    let reply = match decode_image_reply(&response.body, created_seconds) {
        Ok(reply) => ImageGenerationReply {
            usage: usage_identity(reply.usage, &original),
            ..reply
        },
        Err(error) => return Err(ImageVariationSource::Reply(error)),
    };
    Ok(ImageVariationOutcome {
        reply,
        original_response: original,
    })
}

fn response_headers(headers: &http::HeaderMap) -> Result<ImageHeaders, ImageVariationSource> {
    let mut found = ImageHeaders::new();
    for (name, value) in headers {
        let Ok(text) = value.to_str() else {
            return Err(ImageVariationSource::Reply(ImageReplyError::Text {
                field: "status headers",
            }));
        };
        found.insert(name.as_str().to_owned(), text.to_owned());
    }
    Ok(found)
}
