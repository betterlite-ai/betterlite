use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Number;

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
struct UsageFields {
    usage: Option<IndexMap<String, Json>>,
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
        Some(usage) => usage_from_wire(usage, usage_extras(bytes)?)?,
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

// the SDK's model_dump of the reply: the provider's fields as sent, and every declared field the reply left out as null
pub fn original_response(bytes: &[u8]) -> Result<Json, ImageReplyError> {
    let mut fields: IndexMap<String, Json> = serde_json::from_slice(bytes)?;
    let top = [
        "background",
        "data",
        "output_format",
        "quality",
        "size",
        "usage",
    ];
    fill_declared(&mut fields, &top);
    if let Some(Json::List(items)) = fields.get_mut("data") {
        for item in items {
            if let Json::Object(image) = item {
                fill_declared(image, &["b64_json", "revised_prompt", "url"]);
            }
        }
    }
    if let Some(Json::Object(usage)) = fields.get_mut("usage") {
        fill_declared(usage, &["output_tokens_details"]);
    }
    Ok(Json::Object(fields))
}

fn fill_declared(fields: &mut IndexMap<String, Json>, declared: &[&str]) {
    for name in declared {
        fields.entry((*name).to_owned()).or_insert(Json::Null);
    }
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

// the converter keeps the usage keys it does not know, so they stay in the reply
fn usage_extras(bytes: &[u8]) -> Result<IndexMap<String, Json>, ImageReplyError> {
    let found: UsageFields = serde_json::from_slice(bytes)?;
    let mut fields = found.usage.unwrap_or_default();
    for name in [
        "input_tokens",
        "input_tokens_details",
        "output_tokens",
        "total_tokens",
        "prompt_tokens",
        "completion_tokens",
    ] {
        fields.shift_remove(name);
    }
    Ok(fields)
}

fn usage_from_wire(
    usage: WireUsage,
    mut extra: IndexMap<String, Json>,
) -> Result<ImageUsage, ImageReplyError> {
    let input_tokens = usage.input_tokens.unwrap_or(0);
    let output_tokens = usage.output_tokens.unwrap_or(0);
    let total_tokens = match usage.total_tokens {
        Some(total) => total,
        None => input_tokens
            .checked_add(output_tokens)
            .ok_or(ImageReplyError::TotalTokens)?,
    };
    let details = usage.input_tokens_details.unwrap_or_default();
    extra.insert(
        String::from("prompt_tokens_details"),
        details_wrapper(
            &["audio_tokens", "cached_tokens", "video_tokens"],
            Json::Number(Number::from(details.text_tokens)),
            Json::Number(Number::from(details.image_tokens)),
        ),
    );
    if let Some(Json::Object(output)) = extra.get("output_tokens_details") {
        let wrapper = details_wrapper(
            &[
                "accepted_prediction_tokens",
                "audio_tokens",
                "reasoning_tokens",
                "rejected_prediction_tokens",
                "video_tokens",
            ],
            output.get("text_tokens").cloned().unwrap_or(Json::Null),
            output.get("image_tokens").cloned().unwrap_or(Json::Null),
        );
        extra.insert(String::from("completion_tokens_details"), wrapper);
    }
    Ok(ImageUsage {
        input_tokens,
        input_tokens_details: details,
        output_tokens,
        total_tokens,
        prompt_tokens: Some(usage.prompt_tokens.unwrap_or(input_tokens)),
        completion_tokens: Some(usage.completion_tokens.unwrap_or(output_tokens)),
        extra,
    })
}

// the converter's token-details wrapper: the counts it knows, and null for the rest of its fields
fn details_wrapper(nulls: &[&str], text: Json, image: Json) -> Json {
    let mut fields = IndexMap::new();
    for name in nulls {
        fields.insert(String::from(*name), Json::Null);
    }
    fields.insert(String::from("text_tokens"), text);
    fields.insert(String::from("image_tokens"), image);
    Json::Object(fields)
}
