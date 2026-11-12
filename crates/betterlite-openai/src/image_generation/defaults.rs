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
    match family {
        ImageFamily::DallE2 => {
            apply_dall_e_defaults(reply, optional_params, DEFAULT_DALL_E_2_QUALITY)?;
        }
        ImageFamily::DallE3 => {
            apply_dall_e_defaults(reply, optional_params, DEFAULT_DALL_E_3_QUALITY)?;
        }
        ImageFamily::GptImage => apply_gpt_defaults(reply, optional_params)?,
    }
    Ok(())
}

fn apply_dall_e_defaults(
    reply: &mut ImageGenerationReply,
    optional_params: &IndexMap<String, Json>,
    quality: &str,
) -> Result<(), ImageReplyError> {
    reply.size = optional_default(optional_params, "size", DEFAULT_SIZE)?;
    reply.quality = optional_default(optional_params, "quality", quality)?;
    reply.output_format =
        optional_default(optional_params, "output_format", DEFAULT_OUTPUT_FORMAT)?;
    Ok(())
}

// the GPT family keeps a value the provider sent, then a width x height pair, then the option, then its default
fn apply_gpt_defaults(
    reply: &mut ImageGenerationReply,
    optional_params: &IndexMap<String, Json>,
) -> Result<(), ImageReplyError> {
    if reply.size.as_deref().is_none_or(str::is_empty) {
        reply.size = requested_size(optional_params)?;
    }
    if reply.quality.as_deref().is_none_or(str::is_empty) {
        reply.quality = optional_default(optional_params, "quality", DEFAULT_GPT_QUALITY)?;
    }
    if reply.output_format.as_deref().is_none_or(str::is_empty) {
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
        | Some(Json::Null)
        | Some(Json::Bool(_))
        | Some(Json::Text(_))
        | Some(Json::List(_))
        | Some(Json::Object(_)) => None,
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
