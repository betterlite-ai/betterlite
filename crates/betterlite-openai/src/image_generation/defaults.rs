use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::family::ImageFamily;
use super::reply::{
    DEFAULT_DALL_E_2_QUALITY, DEFAULT_DALL_E_3_QUALITY, DEFAULT_GPT_QUALITY, DEFAULT_OUTPUT_FORMAT,
    DEFAULT_SIZE, ImageGenerationReply, ImageReplyError,
};

pub fn apply_family_defaults(
    family: ImageFamily,
    reply: &mut ImageGenerationReply,
    optional_params: &IndexMap<String, Json>,
) -> Result<(), ImageReplyError> {
    let gpt = family == ImageFamily::GptImage;
    let quality = match family {
        ImageFamily::DallE2 => DEFAULT_DALL_E_2_QUALITY,
        ImageFamily::DallE3 => DEFAULT_DALL_E_3_QUALITY,
        ImageFamily::GptImage => DEFAULT_GPT_QUALITY,
    };
    if !gpt || reply.size.as_deref().is_none_or(str::is_empty) {
        reply.size = if gpt {
            requested_size(optional_params)?
        } else {
            optional_default(optional_params, "size", DEFAULT_SIZE)?
        };
    }
    if !gpt || reply.quality.as_deref().is_none_or(str::is_empty) {
        reply.quality = optional_default(optional_params, "quality", quality)?;
    }
    if !gpt || reply.output_format.as_deref().is_none_or(str::is_empty) {
        reply.output_format =
            optional_default(optional_params, "output_format", DEFAULT_OUTPUT_FORMAT)?;
    }
    Ok(())
}

fn requested_size(
    optional_params: &IndexMap<String, Json>,
) -> Result<Option<String>, ImageReplyError> {
    if let (Some(width), Some(height)) = (
        integer_option(optional_params.get("width")),
        integer_option(optional_params.get("height")),
    ) {
        return Ok(Some(format!("{width}x{height}")));
    }
    optional_default(optional_params, "size", DEFAULT_SIZE)
}

// bool is not an integer here, while Python's isinstance would count it as one
fn integer_option(value: Option<&Json>) -> Option<i64> {
    match value {
        Some(Json::Number(number)) => number.as_i64(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

fn optional_default(
    optional_params: &IndexMap<String, Json>,
    field: &'static str,
    default: &str,
) -> Result<Option<String>, ImageReplyError> {
    match optional_params.get(field) {
        None => Ok(Some(default.to_owned())),
        Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ImageReplyError::Text { field })
        }
    }
}
