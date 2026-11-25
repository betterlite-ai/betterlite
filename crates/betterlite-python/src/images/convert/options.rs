use super::ConvertError;
use super::fields::{boolean, strings};
use betterlite_context::media_optional_params::image_generation::{
    ImageGenerationRequest, get_optional_params_image_gen,
};
use betterlite_context::non_default_params::Params;
use betterlite_context::non_default_params::error::MapperError;
use betterlite_context::non_default_params::mapper::{DropPolicy, OptionalParamsMapper};
use betterlite_models::decisions::Json;
use betterlite_openai::image_generation::family::select_image_family;
use betterlite_sdk::call_setup::RequestMap;

const DECLARED_EDIT_PARAMS: &str =
    "background input_fidelity mask n quality response_format size user imageConfig";
// the names filter_out_litellm_params removes, plus the parameters the call handles itself
const CONTROL_PARAMETERS: &str = "additional_drop_params aimg_generation api_base api_key api_version async_call azure_ad_token_provider base_model base_url client custom_llm_provider default_headers deployment_id drop_params extra_headers extra_query headers litellm_call_id litellm_logging_obj logger_fn max_retries metadata mock_response model model_info num_retries organization prompt request_timeout timeout";

pub(crate) fn generation_options(
    arguments: &RequestMap,
    model: &str,
    global_drop_params: bool,
) -> Result<RequestMap, ConvertError> {
    let dropped = strings(arguments, "additional_drop_params")?;
    let drop_policy = DropPolicy {
        drop_params: boolean(arguments, "drop_params")?,
        litellm_drop_params: global_drop_params,
    };
    let mut passed: Params = serde_json::from_slice(&serde_json::to_vec(arguments)?)?;
    passed.retain(|name, _| {
        !CONTROL_PARAMETERS
            .split_whitespace()
            .any(|control| control == name)
    });
    let mapped = get_optional_params_image_gen(&ImageGenerationRequest {
        model: Some(model),
        custom_llm_provider: Some("openai"),
        params: &passed,
        additional_drop_params: &dropped,
        drop_policy,
        provider_config: Some(&GenerationMapper),
        bedrock_config: None,
    })?;
    Ok(serde_json::from_slice(&serde_json::to_vec(&mapped)?)?)
}

struct GenerationMapper;

impl OptionalParamsMapper for GenerationMapper {
    fn supported_openai_params(&self, model: &str) -> Vec<String> {
        select_image_family(model)
            .supported_params()
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    }

    fn map_openai_params(
        &self,
        non_default_params: &Params,
        mut optional_params: Params,
        _model: &str,
        _drop_params: bool,
    ) -> Result<Params, MapperError> {
        // The shared image mapper already checks and drops unsupported keys before this identity mapping.
        optional_params.extend(non_default_params.clone());
        Ok(optional_params)
    }
}

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
    // imageConfig is the only declared edit option OpenAI does not support.
    if selected.shift_remove("imageConfig").is_some() && !global_drop_params && !call_drop {
        return Err(ConvertError::UnsupportedParams {
            model: model.to_owned(),
            params: String::from("imageConfig"),
        });
    }
    Ok(selected)
}

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
