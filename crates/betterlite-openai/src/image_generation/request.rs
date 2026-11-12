use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::call_helpers::headers::{
    ProviderHeaderProjection, set_provider_response_headers,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use super::call::{
    ImageGenerationCall, PreparedImageCall, authorization_headers, prepare_image_call,
};
use super::decode::{decode_image_reply, original_response};
use super::error::{ImageGenerationFailure, ImageGenerationSource, OpenAiImageError};
use super::logging::{ImageGenerationLogger, PostCallRecord, PreCallRecord};
use super::reply::{ImageGenerationReply, ImageReplyError};
use super::transport::ImageGenerationTransport;
use super::wire::outbound_request;
use crate::compatible_wire::error::OpenAiLikeError;

#[derive(Debug)]
pub struct ImageGenerationOutcome {
    pub reply: ImageGenerationReply,
    pub original_response: Json,
    pub provider_headers: ProviderHeaderProjection,
}

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
    // the SDK client refuses to exist without a key, so nothing reaches pre_call
    let Some(api_key) = prepared.api_key.as_ref() else {
        return Err(fail_after_prepared(
            &prepared,
            ImageGenerationSource::MissingApiKey,
            logger,
        ));
    };
    let record = PreCallRecord {
        input: prepared.prompt.clone(),
        headers: authorization_headers(api_key),
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

pub async fn send_image_generation<T>(
    prepared: &PreparedImageCall,
    transport: &T,
) -> Result<ImageGenerationOutcome, ImageGenerationSource>
where
    T: ImageGenerationTransport,
{
    let response = transport
        .generate(outbound_request(prepared)?)
        .await
        .map_err(ImageGenerationSource::Transport)?;
    let mut reply = checked_reply(&response, prepared.now_seconds)?;
    let original_response = original_response(&response.body)?;
    // the SDK model always dumps usage.output_tokens_details, null when the provider sent none
    if let Json::Object(fields) = &original_response
        && let Some(Json::Object(_)) = fields.get("usage")
    {
        let extra = &mut reply.usage.extra;
        extra
            .entry("output_tokens_details".to_owned())
            .or_insert(Json::Null);
    }
    let provider_headers =
        set_provider_response_headers(&IndexMap::new(), &response_headers(&response.headers)?);
    Ok(ImageGenerationOutcome {
        reply,
        original_response,
        provider_headers,
    })
}

pub fn finish_image_generation(
    prepared: &PreparedImageCall,
    sent: Result<ImageGenerationOutcome, ImageGenerationSource>,
    logger: &mut dyn ImageGenerationLogger,
) -> Result<ImageGenerationOutcome, ImageGenerationFailure> {
    match sent {
        Ok(outcome) => {
            logger.post_call(PostCallRecord {
                input: prepared.prompt.clone(),
                original_response: outcome.original_response.clone(),
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
    logger.post_call(PostCallRecord {
        input: prepared.prompt.clone(),
        original_response: Json::Text(source.to_string()),
        data: (!prepared.async_mode).then(|| prepared.data.clone()),
    });
    if prepared.async_mode {
        ImageGenerationFailure::Raw(source)
    } else {
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
        Err(ImageReplyError::ResponseObject { status, message }) => {
            Err(ImageGenerationSource::ResponseObject { status, message })
        }
        Err(error) => Err(ImageGenerationSource::Reply(error)),
    }
}

#[derive(Deserialize)]
struct StatusBody {
    error: Option<StatusError>,
}

#[derive(Deserialize)]
struct StatusError {
    message: Option<String>,
}

fn status_message(text: &str) -> String {
    serde_json::from_str::<StatusBody>(text)
        .ok()
        .and_then(|body| body.error)
        .and_then(|error| error.message)
        .unwrap_or_else(|| text.to_owned())
}

fn response_headers(
    headers: &http::HeaderMap,
) -> Result<IndexMap<String, String>, ImageGenerationSource> {
    let mut found = IndexMap::new();
    for (name, value) in headers {
        let text = value
            .to_str()
            .map_err(|source| ImageGenerationSource::ResponseHeader {
                name: name.as_str().to_owned(),
                source,
            })?;
        found.insert(name.as_str().to_owned(), text.to_owned());
    }
    Ok(found)
}
