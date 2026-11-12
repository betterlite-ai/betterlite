use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::reply::{
    ImageDatum, ImageGenerationReply, ImageReplyError, ImageUsage, ImageUsageDetails,
};

pub fn decode_image_reply(
    bytes: &[u8],
    created_seconds: i64,
) -> Result<ImageGenerationReply, ImageReplyError> {
    let value: Json = serde_json::from_slice(bytes)?;
    let reply = reply_from_json(&value, created_seconds)?;
    Ok(reply)
}

// LiteLLM builds the ImageResponse from a plain dict, so a reply with no created takes the supplied clock
pub fn reply_from_json(
    value: &Json,
    created_seconds: i64,
) -> Result<ImageGenerationReply, ImageReplyError> {
    let Json::Object(fields) = value else {
        return Err(ImageReplyError::Object);
    };
    if let Some(error) = fields.get("error")
        && let Some(failure) = response_object_error(error)
    {
        return Err(failure);
    }
    Ok(ImageGenerationReply {
        created: reply_created(fields.get("created"), created_seconds)?,
        data: reply_data(fields.get("data"))?,
        usage: reply_usage(fields.get("usage"))?,
        background: optional_field(fields.get("background"), "background")?,
        quality: optional_field(fields.get("quality"), "quality")?,
        output_format: optional_field(fields.get("output_format"), "output_format")?,
        size: optional_field(fields.get("size"), "size")?,
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
                .and_then(|code| match code {
                    Json::Number(number) => {
                        number.as_i64().and_then(|value| u16::try_from(value).ok())
                    }
                    Json::Null
                    | Json::Bool(_)
                    | Json::Text(_)
                    | Json::List(_)
                    | Json::Object(_) => None,
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

fn reply_created(created: Option<&Json>, created_seconds: i64) -> Result<i64, ImageReplyError> {
    match created {
        None | Some(Json::Null) => Ok(created_seconds),
        Some(Json::Number(number)) => number.as_i64().ok_or(ImageReplyError::Created),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(ImageReplyError::Created)
        }
    }
}

fn reply_data(data: Option<&Json>) -> Result<Vec<ImageDatum>, ImageReplyError> {
    match data {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::List(items)) => items
            .iter()
            .enumerate()
            .map(|(index, item)| image_datum(index, item))
            .collect(),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(ImageReplyError::Data)
        }
    }
}

fn image_datum(index: usize, value: &Json) -> Result<ImageDatum, ImageReplyError> {
    let Json::Object(fields) = value else {
        return Err(ImageReplyError::Datum { index });
    };
    Ok(ImageDatum {
        b64_json: datum_text(index, "b64_json", fields.get("b64_json"))?,
        url: datum_text(index, "url", fields.get("url"))?,
        revised_prompt: datum_text(index, "revised_prompt", fields.get("revised_prompt"))?,
        provider_specific_fields: match fields.get("provider_specific_fields") {
            None | Some(Json::Null) => None,
            Some(Json::Object(extra)) => Some(extra.clone()),
            Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                return Err(ImageReplyError::DatumFields { index });
            }
        },
    })
}

fn datum_text(
    index: usize,
    field: &'static str,
    value: Option<&Json>,
) -> Result<Option<String>, ImageReplyError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ImageReplyError::DatumText { index, field })
        }
    }
}

fn reply_usage(usage: Option<&Json>) -> Result<ImageUsage, ImageReplyError> {
    match usage {
        None | Some(Json::Null) => Ok(ImageUsage::default()),
        Some(Json::Object(fields)) => usage_from_fields(fields),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(ImageReplyError::Usage)
        }
    }
}

fn usage_from_fields(fields: &IndexMap<String, Json>) -> Result<ImageUsage, ImageReplyError> {
    let input_tokens = usage_count(fields.get("input_tokens"), "input_tokens")?.unwrap_or(0);
    let output_tokens = usage_count(fields.get("output_tokens"), "output_tokens")?.unwrap_or(0);
    let total_tokens = match usage_count(fields.get("total_tokens"), "total_tokens")? {
        Some(total) => total,
        None => input_tokens
            .checked_add(output_tokens)
            .ok_or(ImageReplyError::UsageCount {
                field: "total_tokens",
            })?,
    };
    let details = match fields.get("input_tokens_details") {
        None | Some(Json::Null) => ImageUsageDetails::default(),
        Some(Json::Object(details)) => ImageUsageDetails {
            image_tokens: usage_count(details.get("image_tokens"), "image_tokens")?.ok_or(
                ImageReplyError::UsageDetailCount {
                    field: "image_tokens",
                },
            )?,
            text_tokens: usage_count(details.get("text_tokens"), "text_tokens")?.ok_or(
                ImageReplyError::UsageDetailCount {
                    field: "text_tokens",
                },
            )?,
        },
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(ImageReplyError::UsageDetails);
        }
    };
    let prompt_tokens =
        usage_count(fields.get("prompt_tokens"), "prompt_tokens")?.unwrap_or(input_tokens);
    let completion_tokens =
        usage_count(fields.get("completion_tokens"), "completion_tokens")?.unwrap_or(output_tokens);
    Ok(ImageUsage {
        input_tokens,
        input_tokens_details: details,
        output_tokens,
        total_tokens,
        prompt_tokens: Some(prompt_tokens),
        completion_tokens: Some(completion_tokens),
    })
}

fn usage_count(value: Option<&Json>, field: &'static str) -> Result<Option<i64>, ImageReplyError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => number
            .as_i64()
            .map(Some)
            .ok_or(ImageReplyError::UsageCount { field }),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(ImageReplyError::UsageCount { field })
        }
    }
}

fn optional_field(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<String>, ImageReplyError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ImageReplyError::Text { field })
        }
    }
}
