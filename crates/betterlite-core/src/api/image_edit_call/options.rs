use betterlite_context::non_default_params::Params;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageFile, ImageParams};
use betterlite_openai::image_edits::file::DEFAULT_MASK_NAME;
use serde_json::Value;

use super::call::ImageEditCall;
use super::error::ImageEditError;
use crate::api::image_input_options::options::{
    ImageEditOptionalRequestParams, Quality, ResponseFormat,
};
use crate::api::image_input_options::selection::OptionalParamsPolicy;

// the typed ImageEditOptionalRequestParams names, which arrive as named arguments and never through kwargs
const NAMED_PARAMS: [&str; 6] = ["mask", "n", "quality", "response_format", "size", "user"];

// local_vars.update(kwargs), then get_requested_image_edit_optional_param: typed fields in signature order,
// then every provider-supported kwargs entry in the caller's order
pub(crate) fn requested_options(
    call: &ImageEditCall,
    provider_supported: &[String],
) -> Result<ImageEditOptionalRequestParams, ImageEditError> {
    let mut options = ImageEditOptionalRequestParams {
        mask: call.mask.as_ref().map(mask_name),
        count: call.count,
        quality: call.quality,
        response_format: call.response_format,
        size: call.size.clone(),
        user: call.user.clone(),
        ..ImageEditOptionalRequestParams::default()
    };
    for name in provider_supported {
        if NAMED_PARAMS.contains(&name.as_str()) {
            continue;
        }
        if let Some(value) = call.kwargs.get(name) {
            options
                .extensions
                .insert(name.clone(), serde_json::to_value(value)?);
        }
    }
    Ok(options)
}

// litellm.drop_params is True or the call's drop_params is True; additional_drop_params drops by name
#[must_use]
pub(crate) fn params_policy(
    call: &ImageEditCall,
    global_drop_params: bool,
) -> OptionalParamsPolicy {
    OptionalParamsPolicy {
        global_drop_params,
        call_drop_params: matches!(call.kwargs.get("drop_params"), Some(Json::Bool(true))),
        additional_drop_params: additional_drop_params(call.kwargs.get("additional_drop_params")),
    }
}

fn additional_drop_params(value: Option<&Json>) -> Vec<String> {
    let Some(Json::List(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            Json::Text(name) => Some(name.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        })
        .collect()
}

// The OpenAI config's map_openai_params: a dict copy of the requested options, in the TypedDict's order
#[must_use]
pub(crate) fn image_edit_params(options: &ImageEditOptionalRequestParams) -> ImageParams {
    let mut params = ImageParams::new();
    if let Some(mask) = &options.mask {
        params.insert(String::from("mask"), Json::Text(mask.clone()));
    }
    if let Some(count) = options.count {
        params.insert(
            String::from("n"),
            Json::Number(serde_json::Number::from(count)),
        );
    }
    if let Some(quality) = options.quality {
        params.insert(String::from("quality"), quality_text(quality));
    }
    if let Some(format) = options.response_format {
        params.insert(String::from("response_format"), format_text(format));
    }
    if let Some(size) = &options.size {
        params.insert(String::from("size"), Json::Text(size.clone()));
    }
    if let Some(user) = &options.user {
        params.insert(String::from("user"), Json::Text(user.clone()));
    }
    for (name, value) in &options.extensions {
        params.insert(name.clone(), json_from_value(value));
    }
    params
}

// the name the OpenAI transform uploads a mask file under
fn mask_name(mask: &ImageFile) -> String {
    mask.filename
        .clone()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| String::from(DEFAULT_MASK_NAME))
}

fn quality_text(value: Quality) -> Json {
    match value {
        Quality::High => Json::Text(String::from("high")),
        Quality::Medium => Json::Text(String::from("medium")),
        Quality::Low => Json::Text(String::from("low")),
        Quality::Standard => Json::Text(String::from("standard")),
        Quality::Auto => Json::Text(String::from("auto")),
    }
}

fn format_text(value: ResponseFormat) -> Json {
    match value {
        ResponseFormat::Url => Json::Text(String::from("url")),
        ResponseFormat::B64Json => Json::Text(String::from("b64_json")),
    }
}

#[must_use]
pub(crate) fn json_from_value(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(flag) => Json::Bool(*flag),
        Value::Number(number) => Json::Number(number.clone()),
        Value::String(text) => Json::Text(text.clone()),
        Value::Array(items) => Json::List(items.iter().map(json_from_value).collect()),
        Value::Object(entries) => Json::Object(
            entries
                .iter()
                .map(|(name, item)| (name.clone(), json_from_value(item)))
                .collect(),
        ),
    }
}

// frozenset(supported) ∩ non_default_params read back over local_vars: the caller's own kwarg order
#[must_use]
pub(crate) fn supported_extensions(
    kwargs: &ImageParams,
    non_default: &Params,
    supported: &[String],
) -> Vec<String> {
    kwargs
        .keys()
        .filter(|name| non_default.contains_key(*name))
        .filter(|name| supported.iter().any(|candidate| candidate == *name))
        .cloned()
        .collect()
}
