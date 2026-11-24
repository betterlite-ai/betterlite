use super::ConvertError;
use super::fields::{boolean, strings};
use betterlite_models::decisions::Json;
use betterlite_sdk::call_setup::RequestMap;

// ImageEditOptionalRequestParams' declared fields, in the order get_optional_params_image_edit keeps them
const DECLARED_EDIT_PARAMS: [&str; 9] = [
    "background",
    "input_fidelity",
    "mask",
    "n",
    "quality",
    "response_format",
    "size",
    "user",
    "imageConfig",
];
// generation's named parameters, which pass through beside the kwargs
const GENERATION_PARAMS: [&str; 7] = [
    "imageConfig",
    "n",
    "quality",
    "response_format",
    "size",
    "style",
    "user",
];

// OpenAIImageEditConfig.get_supported_openai_params
fn supported_edit_param(name: &str) -> bool {
    matches!(
        name,
        "image"
            | "prompt"
            | "background"
            | "input_fidelity"
            | "mask"
            | "model"
            | "n"
            | "quality"
            | "response_format"
            | "size"
            | "user"
            | "extra_headers"
            | "extra_query"
            | "extra_body"
            | "timeout"
    )
}

// the names filter_out_litellm_params removes, plus the named parameters the call handles itself
fn control_parameter(name: &str) -> bool {
    matches!(
        name,
        "additional_drop_params"
            | "aimg_generation"
            | "api_base"
            | "api_key"
            | "api_version"
            | "async_call"
            | "azure_ad_token_provider"
            | "base_model"
            | "client"
            | "custom_llm_provider"
            | "deployment_id"
            | "drop_params"
            | "extra_body"
            | "extra_headers"
            | "extra_query"
            | "headers"
            | "litellm_call_id"
            | "litellm_logging_obj"
            | "logger_fn"
            | "max_retries"
            | "metadata"
            | "mock_response"
            | "model"
            | "model_info"
            | "num_retries"
            | "organization"
            | "prompt"
            | "timeout"
    )
}

// get_optional_params_image_gen hands an openai call every non-default value it was given
pub(crate) fn generation_options(arguments: &RequestMap) -> RequestMap {
    let mut options = RequestMap::new();
    for (name, value) in arguments {
        if matches!(value, Json::Null) {
            continue;
        }
        if !GENERATION_PARAMS.contains(&name.as_str()) && control_parameter(name) {
            continue;
        }
        options.insert(name.clone(), value.clone());
    }
    options
}

// ImageEditRequestUtils.get_optional_params_image_edit: declared fields, then additional drops,
// then the unsupported check that only drop_params may silence
pub(crate) fn edit_options(
    arguments: &RequestMap,
    model: &str,
    global_drop_params: bool,
) -> Result<RequestMap, ConvertError> {
    let additional = strings(arguments, "additional_drop_params")?;
    let call_drop = boolean(arguments, "drop_params")?.unwrap_or(false);
    let drop = global_drop_params || call_drop;
    let mut selected = RequestMap::new();
    for name in DECLARED_EDIT_PARAMS {
        let Some(value) = arguments
            .get(name)
            .filter(|value| !matches!(value, Json::Null))
        else {
            continue;
        };
        // the image and the mask are multipart files, and the transform drops both from the data anyway
        if name == "mask" {
            continue;
        }
        selected.insert(name.to_owned(), value.clone());
    }
    for name in &additional {
        selected.shift_remove(name);
    }
    let mut unsupported = Vec::new();
    for name in selected.keys() {
        if !supported_edit_param(name) {
            unsupported.push(name.clone());
        }
    }
    if !unsupported.is_empty() && !drop {
        return Err(ConvertError::UnsupportedParams {
            model: model.to_owned(),
            params: unsupported.join(", "),
        });
    }
    for name in &unsupported {
        selected.shift_remove(name);
    }
    Ok(selected)
}
