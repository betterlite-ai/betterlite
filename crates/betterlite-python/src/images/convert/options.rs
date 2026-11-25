use super::ConvertError;
use super::fields::{boolean, strings};
use betterlite_models::decisions::Json;
use betterlite_openai::image_generation::family::{map_openai_params, select_image_family};
use betterlite_sdk::call_setup::RequestMap;

// ImageEditOptionalRequestParams' declared fields, in the order get_optional_params_image_edit keeps them
const DECLARED_EDIT_PARAMS: &str =
    "background input_fidelity mask n quality response_format size user imageConfig";
// OpenAIImageEditConfig.get_supported_openai_params
const SUPPORTED_EDIT_PARAMS: &str = "image prompt background input_fidelity mask model n quality response_format size user extra_headers extra_query extra_body timeout";
// the names filter_out_litellm_params removes, plus the parameters the call handles itself
const CONTROL_PARAMETERS: &str = "additional_drop_params aimg_generation api_base api_key api_version async_call azure_ad_token_provider base_model base_url client custom_llm_provider default_headers deployment_id drop_params extra_body extra_headers extra_query headers litellm_call_id litellm_logging_obj logger_fn max_retries metadata mock_response model model_info num_retries organization prompt request_timeout timeout";

// get_optional_params_image_gen keeps the model family's supported names and raises for the rest
pub(crate) fn generation_options(
    arguments: &RequestMap,
    model: &str,
    drop_params: bool,
) -> Result<RequestMap, ConvertError> {
    let dropped = strings(arguments, "additional_drop_params")?;
    let mut requested = RequestMap::new();
    for (name, value) in arguments {
        if matches!(value, Json::Null) || dropped.contains(name) {
            continue;
        }
        if CONTROL_PARAMETERS
            .split_whitespace()
            .any(|control| control == name)
        {
            continue;
        }
        requested.insert(name.clone(), value.clone());
    }
    let mut options = RequestMap::new();
    let family = select_image_family(model);
    map_openai_params(family, model, &requested, &mut options, drop_params)
        .map_err(|error| ConvertError::Unsupported(error.to_string()))?;
    Ok(options)
}

// ImageEditRequestUtils.get_optional_params_image_edit runs the declared fields, the additional drops, then the unsupported check
pub(crate) fn edit_options(
    arguments: &RequestMap,
    model: &str,
    global_drop_params: bool,
) -> Result<RequestMap, ConvertError> {
    let additional = strings(arguments, "additional_drop_params")?;
    let call_drop = boolean(arguments, "drop_params")?.unwrap_or(false);
    let mut selected = requested(arguments);
    for name in &additional {
        selected.shift_remove(name);
    }
    let mut unsupported = Vec::new();
    for name in selected.keys() {
        if !SUPPORTED_EDIT_PARAMS
            .split_whitespace()
            .any(|supported| supported == name)
        {
            unsupported.push(name.clone());
        }
    }
    if !unsupported.is_empty() && !global_drop_params && !call_drop {
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

// get_requested_image_edit_optional_param keeps a declared field the caller set; image and mask are multipart files
fn requested(arguments: &RequestMap) -> RequestMap {
    let mut selected = RequestMap::new();
    for name in DECLARED_EDIT_PARAMS.split_whitespace() {
        let Some(value) = arguments
            .get(name)
            .filter(|value| !matches!(value, Json::Null))
        else {
            continue;
        };
        if name != "mask" {
            selected.insert(name.to_owned(), value.clone());
        }
    }
    selected
}
