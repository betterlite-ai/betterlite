use betterlite_context::non_default_params::Params;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageFile, ImageParams};
use betterlite_openai::image_edits::file::DEFAULT_MASK_NAME;
use serde::de::DeserializeOwned;

use super::call::ImageEditCall;
use super::error::ImageEditError;
use crate::api::image_input_options::options::ImageEditOptionalRequestParams;
use crate::api::image_input_options::selection::{OptionalParamsPolicy, requested_optional_params};

// local_vars.update(kwargs) then get_requested_image_edit_optional_param: a kwarg of a declared field fills it, the rest are extensions the provider supports
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
    for (name, value) in &call.kwargs {
        match name.as_str() {
            "background" => options.background = typed_kwarg(name, value)?,
            "input_fidelity" => options.input_fidelity = typed_kwarg(name, value)?,
            "imageConfig" => options.image_config = typed_kwarg(name, value)?,
            _ => {
                options
                    .extensions
                    .insert(name.clone(), serde_json::to_value(value)?);
            }
        }
    }
    Ok(requested_optional_params(&options, provider_supported))
}

// a declared field a kwarg fills: JSON null counts as absent, and a value the declared type can't hold names the field
fn typed_kwarg<T: DeserializeOwned>(name: &str, value: &Json) -> Result<Option<T>, ImageEditError> {
    match value {
        Json::Null => Ok(None),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            serde_json::from_value(serde_json::to_value(value)?)
                .map(Some)
                .map_err(|error| ImageEditError::Invalid(format!("{name}: {error}")))
        }
    }
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

// the name the OpenAI transform uploads a mask file under
fn mask_name(mask: &ImageFile) -> String {
    mask.filename
        .clone()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| String::from(DEFAULT_MASK_NAME))
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
