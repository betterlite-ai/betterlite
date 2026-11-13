use betterlite_models::controls::json_value::JsonValue;

use super::Params;
use super::error::NonDefaultParamsError;
use super::extra_body::{ProviderSpecificRequest, add_provider_specific_params};
use super::filters::select_non_default_params;
use super::mapper::OptionalParamsMapper;
use super::names::DEFAULT_CHAT_COMPLETION_PARAMS;

const NEVER_NON_DEFAULT: &[&str] = &[
    "model",
    "custom_llm_provider",
    "api_version",
    "drop_params",
    "allowed_openai_params",
    "additional_drop_params",
];

const SENSITIVE_KEY_PHRASES: &[&str] = &["key", "secret", "access", "credential"];

pub struct PreProcessRequest<'request> {
    pub passed_params: &'request Params,
    pub special_params: &'request Params,
    pub custom_llm_provider: &'request str,
    pub additional_drop_params: &'request [String],
    pub provider_config: Option<&'request dyn OptionalParamsMapper>,
}

#[derive(Clone, Copy)]
pub struct PreProcessOptions {
    pub remove_sensitive_keys: bool,
    pub add_provider_specific_params: bool,
}

pub struct PreProcessed {
    pub passed_params: Params,
    pub non_default_params: Params,
}

fn special_param_is_dropped(key: &str, custom_llm_provider: &str) -> bool {
    match key {
        "aws_bedrock_project_id" => true,
        "hf_model_name" => custom_llm_provider != "sagemaker",
        _ if key.starts_with("aws_") => {
            custom_llm_provider != "bedrock" && !custom_llm_provider.starts_with("sagemaker")
        }
        _ if key.starts_with("vertex_") => {
            !matches!(custom_llm_provider, "vertex_ai" | "vertex_ai_beta" | "gdc")
        }
        _ => false,
    }
}

#[must_use]
pub fn base_pre_process_params(
    request: &PreProcessRequest<'_>,
    default_names: &str,
    endpoint_specific: &[&str],
) -> PreProcessed {
    let mut passed_params = request.passed_params.clone();
    for (key, value) in request.special_params {
        if !special_param_is_dropped(key, request.custom_llm_provider) {
            passed_params.insert(key.clone(), value.clone());
        }
    }
    let mut candidates = passed_params.clone();
    candidates.retain(|key, _| {
        !NEVER_NON_DEFAULT.contains(&key.as_str()) && !endpoint_specific.contains(&key.as_str())
    });
    let names: Vec<&str> = default_names.split_whitespace().collect();
    let non_default_params =
        select_non_default_params(&candidates, &names, request.additional_drop_params);
    PreProcessed {
        passed_params,
        non_default_params,
    }
}

// LiteLLM's tools branch tests `isinstance(non_default_params, list)` on a dict, so the additionalProperties cleanup never runs.
pub fn pre_process_non_default_params(
    request: &PreProcessRequest<'_>,
    options: PreProcessOptions,
) -> Result<Params, NonDefaultParamsError> {
    let processed = base_pre_process_params(request, DEFAULT_CHAT_COMPLETION_PARAMS, &["messages"]);
    let mut non_default_params = processed.non_default_params;
    if let Some(format) = non_default_params.get_mut("response_format") {
        if !matches!(format, JsonValue::Object(_)) {
            return Err(NonDefaultParamsError::WrongType(
                "response_format",
                "an object",
            ));
        }
        if let Some(config) = request.provider_config {
            config.map_response_format(format);
        }
    }
    if options.add_provider_specific_params {
        let openai_params: Vec<&str> = DEFAULT_CHAT_COMPLETION_PARAMS.split_whitespace().collect();
        non_default_params = add_provider_specific_params(
            non_default_params,
            &ProviderSpecificRequest {
                passed_params: &processed.passed_params,
                custom_llm_provider: request.custom_llm_provider,
                openai_params: &openai_params,
                additional_drop_params: request.additional_drop_params,
            },
        )?;
    }
    if options.remove_sensitive_keys {
        remove_sensitive_keys(&mut non_default_params);
    }
    Ok(non_default_params)
}

pub fn remove_sensitive_keys(params: &mut Params) {
    params.retain(|key, _| {
        let lowered = key.to_lowercase();
        !SENSITIVE_KEY_PHRASES
            .iter()
            .any(|phrase| lowered.contains(phrase))
    });
}
