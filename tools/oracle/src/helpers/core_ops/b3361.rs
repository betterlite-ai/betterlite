use betterlite_core::containers::container_request_parameters::{
    ContainerFieldTypeError,
    ContainerProviderConfig,
    // ast-grep-ignore: no-long-names
    decode_managed_container_id_for_request as decode_request,
    // ast-grep-ignore: no-long-names
    encode_container_id_in_response as encode_response_id,
    // ast-grep-ignore: no-long-names
    get_optional_params_container_create as map_create_params,
    get_provider_container_config,
    // ast-grep-ignore: no-long-names
    get_requested_container_create_optional_param as extract_create_params,
    // ast-grep-ignore: no-long-names
    get_requested_container_list_optional_param as extract_list_params,
};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::api::container_contracts::records::{
    ContainerCreateRequestParams, ContainerListOptionalParams,
};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
struct DecodeCall {
    container_id: String,
    custom_llm_provider: String,
    #[serde(default)]
    litellm_params: IndexMap<String, JsonValue>,
}

#[derive(Debug, Deserialize)]
struct CreateOptionalCall {
    #[serde(default)]
    passed_params: ContainerCreateRequestParams,
}

#[derive(Debug, Deserialize)]
struct FilterCall {
    provider: String,
    #[serde(default)]
    passed_params: ContainerCreateRequestParams,
}

#[derive(Debug, Deserialize)]
struct ListOptionalCall {
    #[serde(default)]
    passed_params: ContainerListOptionalParams,
}

#[derive(Debug, Deserialize)]
struct EncodeCall {
    container_id: String,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    litellm_metadata: Option<IndexMap<String, JsonValue>>,
    #[serde(default)]
    extra_body: Option<IndexMap<String, JsonValue>>,
}

#[derive(Debug, Deserialize)]
struct ProviderConfigCall {
    custom_llm_provider: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    DecodeManagedId(DecodeCall),
    CreateOptionalParams(CreateOptionalCall),
    CreateParams(FilterCall),
    ListOptionalParams(ListOptionalCall),
    EncodeContainerId(EncodeCall),
    ProviderConfig(ProviderConfigCall),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::DecodeManagedId(call) => decode(call),
            Input::CreateOptionalParams(call) => Ok(json!({
                "params": extract_create_params(&call.passed_params),
            })),
            Input::CreateParams(call) => filter(call),
            Input::ListOptionalParams(call) => Ok(json!({
                "params": extract_list_params(&call.passed_params),
            })),
            Input::EncodeContainerId(call) => encode(call),
            Input::ProviderConfig(call) => Ok(json!({
                "config": get_provider_container_config(&call.custom_llm_provider)
                    .map(config_name),
            })),
        }
    }
}

fn decode(call: &DecodeCall) -> HelperResult {
    let decoded = decode_request(
        &call.container_id,
        &call.custom_llm_provider,
        &call.litellm_params,
    )
    .map_err(|error| field_error(&error))?;
    Ok(json!({
        "container_id": decoded.container_id,
        "custom_llm_provider": decoded.custom_llm_provider,
        "model_id": decoded.litellm_params.get("model_id"),
    }))
}

fn filter(call: &FilterCall) -> HelperResult {
    if get_provider_container_config(&call.provider).is_none() {
        return Err(Box::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: format!("{} is not a known provider", call.provider),
        }));
    }
    let optional_params = extract_create_params(&call.passed_params);
    let params = map_create_params(&optional_params);
    Ok(json!({"params": params}))
}

fn encode(call: &EncodeCall) -> HelperResult {
    let encoded = encode_response_id(
        &call.container_id,
        call.custom_llm_provider.as_deref(),
        call.litellm_metadata.as_ref(),
        call.extra_body.as_ref(),
    )
    .map_err(|error| field_error(&error))?;
    Ok(json!({"id": encoded}))
}

fn field_error(error: &ContainerFieldTypeError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ContainerFieldTypeError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

const fn config_name(config: ContainerProviderConfig) -> &'static str {
    match config {
        ContainerProviderConfig::OpenAi => "OpenAIContainerConfig",
        ContainerProviderConfig::Azure => "AzureContainerConfig",
    }
}
