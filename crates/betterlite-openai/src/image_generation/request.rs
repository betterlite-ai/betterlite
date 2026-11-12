use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::call_helpers::headers::{
    ProviderHeaderProjection, set_provider_response_headers,
};
use betterlite_models::decisions::Json;
use http::Method;
use indexmap::IndexMap;

use super::call::{
    ImageGenerationCall, PreparedImageCall, authorization_headers, prepare_image_call,
};
use super::error::{ImageGenerationFailure, ImageGenerationSource, OpenAiImageError};
use super::logging::{ImageGenerationLogger, PostCallRecord, PreCallRecord};
use super::reply::{ImageGenerationReply, ImageReplyError, decode_image_reply};
use super::transport::ImageGenerationTransport;
use crate::compatible_wire::error::OpenAiLikeError;

const DEFAULT_TIMEOUT_SECONDS: u64 = 600;

pub struct ImageGenerationOutcome {
    pub reply: ImageGenerationReply,
    pub provider_headers: ProviderHeaderProjection,
}

// the sync facade's build step: the body is ready and the pre-call record has gone to the logging object
pub fn begin_image_generation(
    call: ImageGenerationCall,
    logger: &mut dyn ImageGenerationLogger,
) -> Result<PreparedImageCall, ImageGenerationFailure> {
    let prompt = call.prompt.clone();
    let prepared = match prepare_image_call(call) {
        Ok(prepared) => prepared,
        Err(failure) => {
            logger.post_call(PostCallRecord {
                input: prompt,
                original_response: Json::Text(failure.source.to_string()),
                data: Some(failure.data),
            });
            return Err(ImageGenerationFailure::Wrapped(OpenAiImageError::wrapped(
                failure.source,
            )));
        }
    };
    let record = PreCallRecord {
        input: prepared.prompt.clone(),
        headers: authorization_headers(prepared.api_key.as_ref()),
        api_base: prepared.api_base.clone(),
        data: prepared.data.clone(),
    };
    if let Err(source) = logger.pre_call(record) {
        return Err(fail_after_prepared(
            &prepared,
            ImageGenerationSource::PreCall(source),
            logger,
        ));
    }
    Ok(prepared)
}

// the one transport path both source methods share; it never touches the logging object
pub async fn send_image_generation<T: ImageGenerationTransport>(
    prepared: &PreparedImageCall,
    transport: &T,
) -> Result<ImageGenerationOutcome, ImageGenerationSource> {
    let body = serde_json::to_vec(&prepared.wire_data).map_err(ImageGenerationSource::Encode)?;
    let request = OutboundRequest {
        method: Method::POST,
        url: prepared.url.clone(),
        query: Vec::new(),
        headers: prepared.headers.clone(),
        body: RequestBody::Json(body),
        deadline: prepared
            .timeout
            .unwrap_or(Duration::from_secs(DEFAULT_TIMEOUT_SECONDS)),
    };
    let response = transport
        .generate(request)
        .await
        .map_err(ImageGenerationSource::Transport)?;
    let reply = checked_reply(&response, prepared.now_seconds)?;
    let provider_headers =
        set_provider_response_headers(&IndexMap::new(), &response_headers(&response.headers)?);
    Ok(ImageGenerationOutcome {
        reply,
        provider_headers,
    })
}

// the post-call record and the failure the facade hands back: wrapped for sync, original for async
pub fn finish_image_generation(
    prepared: &PreparedImageCall,
    sent: Result<ImageGenerationOutcome, ImageGenerationSource>,
    logger: &mut dyn ImageGenerationLogger,
) -> Result<ImageGenerationOutcome, ImageGenerationFailure> {
    match sent {
        Ok(outcome) => {
            let original = serde_json::to_value(&outcome.reply)
                .map_err(ImageGenerationSource::Encode)
                .and_then(|value| {
                    serde_json::from_value(value).map_err(ImageGenerationSource::Encode)
                })
                .map_err(|source| {
                    ImageGenerationFailure::Wrapped(OpenAiImageError::wrapped(source))
                })?;
            logger.post_call(PostCallRecord {
                input: prepared.prompt.clone(),
                original_response: original,
                data: Some(prepared.data.clone()),
            });
            Ok(outcome)
        }
        Err(source) => Err(fail_after_prepared(prepared, source, logger)),
    }
}

fn fail_after_prepared(
    prepared: &PreparedImageCall,
    source: ImageGenerationSource,
    logger: &mut dyn ImageGenerationLogger,
) -> ImageGenerationFailure {
    let original = Json::Text(source.to_string());
    if prepared.async_mode {
        logger.post_call(PostCallRecord {
            input: prepared.prompt.clone(),
            original_response: original,
            data: None,
        });
        ImageGenerationFailure::Raw(source)
    } else {
        logger.post_call(PostCallRecord {
            input: prepared.prompt.clone(),
            original_response: original,
            data: Some(prepared.data.clone()),
        });
        ImageGenerationFailure::Wrapped(OpenAiImageError::wrapped(source))
    }
}

fn checked_reply(
    response: &UpstreamResponse,
    created_seconds: i64,
) -> Result<ImageGenerationReply, ImageGenerationSource> {
    if !(200..=299).contains(&response.status) {
        let text = String::from_utf8(response.body.clone()).map_err(|source| {
            ImageGenerationSource::StatusBody {
                status: response.status,
                source,
            }
        })?;
        return Err(ImageGenerationSource::Upstream(OpenAiLikeError {
            status_code: response.status,
            message: status_message(&text),
        }));
    }
    match decode_image_reply(&response.body, created_seconds) {
        Ok(reply) => Ok(reply),
        // the converter raises an exception that carries the error object's status
        Err(ImageReplyError::ResponseObject { status, message }) => {
            Err(ImageGenerationSource::ResponseObject { status, message })
        }
        Err(error) => Err(ImageGenerationSource::Reply(error)),
    }
}

fn status_message(text: &str) -> String {
    let error = serde_json::from_str::<Json>(text)
        .ok()
        .and_then(|value| match value {
            Json::Object(fields) => fields.get("error").cloned(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        });
    error
        .and_then(|error| match error {
            Json::Object(fields) => match fields.get("message") {
                Some(Json::Text(message)) => Some(message.clone()),
                Some(
                    Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
                )
                | None => None,
            },
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        })
        .unwrap_or_else(|| text.to_owned())
}

fn response_headers(
    headers: &http::HeaderMap,
) -> Result<IndexMap<String, String>, ImageGenerationSource> {
    headers
        .iter()
        .map(|(name, value)| {
            value.to_str().map_or_else(
                |source| {
                    Err(ImageGenerationSource::ResponseHeader {
                        name: name.as_str().to_owned(),
                        source,
                    })
                },
                |text| Ok((name.as_str().to_owned(), text.to_owned())),
            )
        })
        .collect()
}
