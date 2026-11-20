use betterlite_azure::passthrough::config::AzurePassthroughConfig;
use betterlite_config::sections::settings::JsonValue;
use betterlite_context::aws_partition::{contains_bedrock_arn, get_aws_partition};
use betterlite_context::bedrock_model::runtime_arn::encode_runtime_modelid_arn;
use betterlite_context::resource_config_selection::passthrough::PassthroughConfigTag;
use betterlite_foundry::passthrough::plan::FoundryPassthroughPlan;
use betterlite_openai::gigachat_passthrough::GigachatPassthroughConfig;
use betterlite_openai::nvidia_nim_passthrough::NvidiaNimPassthroughConfig;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::BasePassthroughConfig;
use betterlite_openai::passthrough::transformation::types::PassthroughUrlRequest;
use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_passthrough::WatsonxPassthroughConfig;
use regex::{NoExpand, Regex};
use url::Url;

use super::config::{ConfigError, PassthroughCallConfig};

// VLLM's ValueError text, braces and all: LiteLLM never formats the placeholder
const VLLM_MISSING_BASE: &str = "VLLM_API_BASE is not set. Please set the environment variable, to use VLLM's pass-through - `{LITELLM_API_BASE}/vllm/{endpoint}`.";
const MODEL_PATH_PATTERN: &str = "model/[^/]+/";
const MANTLE_HOST_PATTERN: &str = r"^https?://bedrock-mantle\.([^/.]+)\.api\.aws(?:/|$)";
const DEFAULT_BEDROCK_REGION: &str = "us-west-2";
const DEFAULT_MANTLE_REGION: &str = "us-east-1";

pub(crate) fn complete_url(
    config: &PassthroughCallConfig,
    request: &PassthroughUrlRequest<'_>,
) -> Result<(Url, String), ConfigError> {
    match config.tag {
        PassthroughConfigTag::VLLM => vllm_url(config, request),
        PassthroughConfigTag::Bedrock => bedrock_url(config, request, false),
        PassthroughConfigTag::BedrockMantle => bedrock_url(config, request, true),
        PassthroughConfigTag::Azure => azure_url(request),
        PassthroughConfigTag::AzureAI => azure_ai_url(request),
        PassthroughConfigTag::GigaChat => gigachat_url(request),
        PassthroughConfigTag::Watsonx => watsonx_url(request),
        PassthroughConfigTag::NvidiaNim => nvidia_nim_url(request),
    }
}

// the merged per-provider configs own their URL rules, so this call only hands them the resolved request
fn azure_url(request: &PassthroughUrlRequest<'_>) -> Result<(Url, String), ConfigError> {
    let config: AzurePassthroughConfig = AzurePassthroughConfig::default();
    BasePassthroughConfig::get_complete_url(&config, request)
        .map_err(|error| ConfigError::message(500, error.to_string()))
}

fn azure_ai_url(request: &PassthroughUrlRequest<'_>) -> Result<(Url, String), ConfigError> {
    let plan: FoundryPassthroughPlan = FoundryPassthroughPlan {
        global_api_base: None,
        global_api_key: None,
        azure: AzurePassthroughConfig::default(),
        ocr: None,
        litellm_params: request.litellm_params.clone(),
    };
    BasePassthroughConfig::get_complete_url(&plan, request)
        .map_err(|error| ConfigError::message(500, error.to_string()))
}

fn gigachat_url(request: &PassthroughUrlRequest<'_>) -> Result<(Url, String), ConfigError> {
    let config = GigachatPassthroughConfig::default();
    BasePassthroughConfig::get_complete_url(&config, request)
        .map_err(|error| ConfigError::message(500, error.to_string()))
}

fn watsonx_url(request: &PassthroughUrlRequest<'_>) -> Result<(Url, String), ConfigError> {
    let environment = WatsonxEnvironment::default();
    let config = WatsonxPassthroughConfig {
        environment: &environment,
    };
    BasePassthroughConfig::get_complete_url(&config, request)
        .map_err(|error| ConfigError::from_status(error.status(), error.to_string()))
}

fn nvidia_nim_url(request: &PassthroughUrlRequest<'_>) -> Result<(Url, String), ConfigError> {
    let config = NvidiaNimPassthroughConfig::default();
    BasePassthroughConfig::get_complete_url(&config, request)
        .map_err(|error| ConfigError::message(500, error.to_string()))
}

// VLLM has no merged config in core's dependency graph; its rule is api base + the native endpoint
fn vllm_url(
    config: &PassthroughCallConfig,
    request: &PassthroughUrlRequest<'_>,
) -> Result<(Url, String), ConfigError> {
    let Some(base) = request.api_base.filter(|base| !base.is_empty()) else {
        return Err(ConfigError::value_error(VLLM_MISSING_BASE.to_owned()));
    };
    let url = config.format_url(request.target.endpoint, base, request.request_query_params)?;
    Ok((url, base.to_owned()))
}

// Bedrock and Bedrock Mantle share the runtime hop; Mantle only lends its region and its bearer token
fn bedrock_url(
    config: &PassthroughCallConfig,
    request: &PassthroughUrlRequest<'_>,
    mantle: bool,
) -> Result<(Url, String), ConfigError> {
    let params = request.litellm_params;
    let region = if mantle {
        mantle_region(params)?
    } else {
        bedrock_region(params, request.target.model)?
    };
    let configured = text_param(params, "aws_bedrock_runtime_endpoint")?;
    let base = runtime_endpoint(request.api_base, configured, &region, mantle);
    let endpoint = match text_param(params, "model_id")? {
        Some(model_id) => rewrite_model_path(request.target.endpoint, &encoded_model_id(model_id)),
        None => request.target.endpoint.to_owned(),
    };
    let url = config.format_url(&endpoint, &base, request.request_query_params)?;
    Ok((url, base))
}

fn runtime_endpoint(
    api_base: Option<&str>,
    configured: Option<&str>,
    region: &str,
    mantle: bool,
) -> String {
    let usable = api_base.filter(|base| !mantle || mantle_region_from_host(base).is_none());
    usable
        .map(str::to_owned)
        .or_else(|| configured.map(str::to_owned))
        .unwrap_or_else(|| default_runtime_endpoint(region))
}

fn default_runtime_endpoint(region: &str) -> String {
    let suffix = get_aws_partition(Some(region)).dns_suffix;
    format!("https://bedrock-runtime.{region}.{suffix}")
}

// _get_aws_region_name: the explicit region is validated, then the model ARN lends one, then us-west-2
fn bedrock_region(params: &Object, model: &str) -> Result<String, ConfigError> {
    let explicit = text_param(params, "aws_region_name")?;
    validate_region(explicit)?;
    if let Some(region) = explicit {
        return Ok(region.to_owned());
    }
    let model_id = text_param(params, "model_id")?;
    if let Some(region) = region_from_model_arn(model_id.unwrap_or(model)) {
        return Ok(region);
    }
    Ok(DEFAULT_BEDROCK_REGION.to_owned())
}

// resolve_mantle_region: the explicit region, then the region in the Mantle api base, then us-east-1
fn mantle_region(params: &Object) -> Result<String, ConfigError> {
    let explicit = text_param(params, "aws_region_name")?;
    validate_region(explicit)?;
    if let Some(region) = explicit {
        return Ok(region.to_owned());
    }
    let base = text_param(params, "api_base")?.filter(|base| !base.is_empty());
    if let Some(region) = base.and_then(mantle_region_from_host) {
        return Ok(region);
    }
    Ok(DEFAULT_MANTLE_REGION.to_owned())
}

fn validate_region(explicit: Option<&str>) -> Result<(), ConfigError> {
    let Some(region) = explicit else {
        return Ok(());
    };
    if is_region(region) {
        return Ok(());
    }
    Err(ConfigError::value_error(format!(
        "Invalid AWS region format: '{region}'. Region names must contain only lowercase letters, digits, and hyphens."
    )))
}

fn is_region(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn region_from_model_arn(model: &str) -> Option<String> {
    if !contains_bedrock_arn(model) {
        return None;
    }
    let region = model.split(':').nth(3)?;
    is_region(region).then(|| region.to_owned())
}

fn mantle_region_from_host(base: &str) -> Option<String> {
    let pattern = Regex::new(MANTLE_HOST_PATTERN).ok()?;
    let captures = pattern.captures(base.trim_end_matches('/'))?;
    captures.get(1).map(|region| region.as_str().to_owned())
}

// BedrockPassthroughConfig._encode_model_id_for_endpoint: the ARN is percent-encoded and read back out
fn encoded_model_id(model_id: &str) -> String {
    let probe = format!("/model/{model_id}/converse");
    let encoded = encode_runtime_modelid_arn(&probe);
    let segment = encoded.split('/').nth(2).filter(|part| !part.is_empty());
    segment.unwrap_or(model_id).to_owned()
}

fn rewrite_model_path(endpoint: &str, encoded: &str) -> String {
    let Some(pattern) = Regex::new(MODEL_PATH_PATTERN).ok() else {
        return endpoint.to_owned();
    };
    let replacement = format!("model/{encoded}/");
    pattern
        .replace_all(endpoint, NoExpand(&replacement))
        .into_owned()
}

// a litellm_params value Python would compare against None, refused when it cannot be a string
fn text_param<'params>(
    params: &'params Object,
    name: &str,
) -> Result<Option<&'params str>, ConfigError> {
    match params.get(name) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(text)) => Ok(Some(text)),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_),
        ) => Err(ConfigError::value_error(format!(
            "litellm_params.{name} must be a string"
        ))),
    }
}
