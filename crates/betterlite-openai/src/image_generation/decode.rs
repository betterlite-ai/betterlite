use betterlite_models::decisions::Json;
use serde::Deserialize;

use super::reply::{
    ImageDatum, ImageGenerationReply, ImageReplyError, ImageUsage, ImageUsageDetails,
};

#[derive(Deserialize)]
struct ErrorProbe {
    error: Option<Json>,
}

#[derive(Deserialize)]
struct WireReply {
    created: Option<i64>,
    data: Option<Vec<ImageDatum>>,
    usage: Option<WireUsage>,
    background: Option<String>,
    quality: Option<String>,
    output_format: Option<String>,
    size: Option<String>,
}

#[derive(Deserialize)]
struct WireUsage {
    input_tokens: Option<i64>,
    input_tokens_details: Option<ImageUsageDetails>,
    output_tokens: Option<i64>,
    total_tokens: Option<i64>,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
}

// LiteLLM builds the ImageResponse from a plain dict, so a reply with no created takes the supplied clock
pub fn decode_image_reply(
    bytes: &[u8],
    created_seconds: i64,
) -> Result<ImageGenerationReply, ImageReplyError> {
    let probe: ErrorProbe = serde_json::from_slice(bytes)?;
    if let Some(error) = &probe.error
        && let Some(failure) = response_object_error(error)
    {
        return Err(failure);
    }
    let wire: WireReply = serde_json::from_slice(bytes)?;
    let usage = match wire.usage {
        Some(usage) => usage_from_wire(usage)?,
        None => ImageUsage::default(),
    };
    Ok(ImageGenerationReply {
        created: wire.created.unwrap_or(created_seconds),
        data: wire.data.unwrap_or_default(),
        usage,
        background: wire.background,
        quality: wire.quality,
        output_format: wire.output_format,
        size: wire.size,
    })
}

fn response_object_error(error: &Json) -> Option<ImageReplyError> {
    match error {
        Json::Null => None,
        Json::Object(fields) => {
            if !message_meaningful(fields.get("message"))
                && fields
                    .get("code")
                    .is_none_or(|code| matches!(code, Json::Null))
            {
                return None;
            }
            let status = fields
                .get("code")
                .and_then(|code| {
                    let Json::Number(number) = code else {
                        return None;
                    };
                    let value = number.as_i64()?;
                    u16::try_from(value).ok()
                })
                .unwrap_or(422);
            let message = match fields.get("message") {
                Some(Json::Text(text)) => text.clone(),
                Some(other) => serde_json::to_string(other).unwrap_or_default(),
                None => String::from("Error in response object"),
            };
            Some(ImageReplyError::ResponseObject { status, message })
        }
        Json::Text(text) if text.is_empty() => None,
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Some(ImageReplyError::ResponseObject {
                status: 422,
                message: String::from("Error in response object"),
            })
        }
    }
}

// an empty error object on a successful reply is how some compatible hosts report no error at all
fn message_meaningful(message: Option<&Json>) -> bool {
    match message {
        None | Some(Json::Null) => false,
        Some(Json::Text(text)) => !text.is_empty(),
        Some(Json::List(items)) => !items.is_empty(),
        Some(Json::Object(fields)) => !fields.is_empty(),
        Some(Json::Bool(flag)) => *flag,
        Some(Json::Number(number)) => number.as_i64().is_none_or(|value| value != 0),
    }
}

fn usage_from_wire(usage: WireUsage) -> Result<ImageUsage, ImageReplyError> {
    let input_tokens = usage.input_tokens.unwrap_or(0);
    let output_tokens = usage.output_tokens.unwrap_or(0);
    let total_tokens = match usage.total_tokens {
        Some(total) => total,
        None => input_tokens
            .checked_add(output_tokens)
            .ok_or(ImageReplyError::TotalTokens)?,
    };
    Ok(ImageUsage {
        input_tokens,
        input_tokens_details: usage.input_tokens_details.unwrap_or_default(),
        output_tokens,
        total_tokens,
        prompt_tokens: Some(usage.prompt_tokens.unwrap_or(input_tokens)),
        completion_tokens: Some(usage.completion_tokens.unwrap_or(output_tokens)),
    })
}
