use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::api::container_contracts::records::{
    ContainerCreateOptionalParams, ContainerCreateRequestParams, ContainerListOptionalParams,
};
use betterlite_openai::containers::requests::SUPPORTED_PARAMS;
use indexmap::IndexMap;
use thiserror::Error;

use crate::api::response_id_codecs::container::{build_container_id, decode_container_id};

// LiteLLM hands back a provider config object here; core names the provider and its caller above builds that config
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerProviderConfig {
    OpenAi,
    Azure,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("the {field} field has the wrong type")]
pub struct ContainerFieldTypeError {
    pub field: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedContainerRequest {
    pub container_id: String,
    pub custom_llm_provider: String,
    pub litellm_params: IndexMap<String, JsonValue>,
}

// LiteLLM takes the LlmProviders enum here; an unknown provider is the caller's ValueError and this lookup only answers None
#[must_use]
pub fn get_provider_container_config(custom_llm_provider: &str) -> Option<ContainerProviderConfig> {
    match custom_llm_provider {
        "openai" => Some(ContainerProviderConfig::OpenAi),
        "azure" | "azure_text" => Some(ContainerProviderConfig::Azure),
        _ => None,
    }
}

// ast-grep-ignore: no-long-names
pub fn decode_managed_container_id_for_request(
    container_id: &str,
    custom_llm_provider: &str,
    litellm_params: &IndexMap<String, JsonValue>,
) -> Result<DecodedContainerRequest, ContainerFieldTypeError> {
    let decoded = decode_container_id(container_id);
    let original_container_id = decoded
        .response_id
        .unwrap_or_else(|| container_id.to_owned());
    let resolved_provider = match decoded.custom_llm_provider.flatten() {
        Some(provider) if custom_llm_provider == "openai" => provider,
        _ => custom_llm_provider.to_owned(),
    };
    let mut updated_params = litellm_params.clone();
    if let Some(model_id) = decoded.model_id.flatten() {
        let preset = match updated_params.get("model_id") {
            None | Some(JsonValue::Null) => false,
            Some(JsonValue::Text(_)) => true,
            Some(
                JsonValue::Bool(_)
                | JsonValue::Number(_)
                | JsonValue::Array(_)
                | JsonValue::Object(_),
            ) => {
                return Err(ContainerFieldTypeError {
                    field: "litellm_params.model_id",
                });
            }
        };
        if !preset {
            updated_params.insert(String::from("model_id"), JsonValue::Text(model_id));
        }
    }
    Ok(DecodedContainerRequest {
        container_id: original_container_id,
        custom_llm_provider: resolved_provider,
        litellm_params: updated_params,
    })
}

// passed_params is LiteLLM's local_vars, whose other keys this whitelist drops
#[must_use]
// ast-grep-ignore: no-long-names
pub fn get_requested_container_create_optional_param(
    passed_params: &ContainerCreateRequestParams,
) -> ContainerCreateOptionalParams {
    ContainerCreateOptionalParams {
        expires_after: passed_params.expires_after.clone(),
        file_ids: passed_params.file_ids.clone(),
        extra_headers: passed_params.extra_headers.clone(),
        extra_body: passed_params.extra_body.clone(),
    }
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn get_optional_params_container_create(
    optional_params: &ContainerCreateOptionalParams,
) -> ContainerCreateOptionalParams {
    // Both providers' supported_params are the same list, and OpenAI's map_openai_params
    // returns the filtered params unchanged (Azure inherits it)
    let supported = &SUPPORTED_PARAMS;
    ContainerCreateOptionalParams {
        expires_after: optional_params
            .expires_after
            .clone()
            .filter(|_| supported.contains(&"expires_after")),
        file_ids: optional_params
            .file_ids
            .clone()
            .filter(|_| supported.contains(&"file_ids")),
        extra_headers: optional_params
            .extra_headers
            .clone()
            .filter(|_| supported.contains(&"extra_headers")),
        extra_body: optional_params
            .extra_body
            .clone()
            .filter(|_| supported.contains(&"extra_body")),
    }
}

// passed_params is LiteLLM's local_vars, whose other keys this whitelist drops
#[must_use]
// ast-grep-ignore: no-long-names
pub fn get_requested_container_list_optional_param(
    passed_params: &ContainerListOptionalParams,
) -> ContainerListOptionalParams {
    ContainerListOptionalParams {
        after: passed_params.after.clone(),
        limit: passed_params.limit,
        order: passed_params.order.clone(),
        extra_headers: passed_params.extra_headers.clone(),
        extra_query: passed_params.extra_query.clone(),
    }
}

// ast-grep-ignore: no-long-names
pub fn encode_container_id_in_response(
    container_id: &str,
    custom_llm_provider: Option<&str>,
    litellm_metadata: Option<&IndexMap<String, JsonValue>>,
    extra_body: Option<&IndexMap<String, JsonValue>>,
) -> Result<String, ContainerFieldTypeError> {
    if let Some(model_id) = model_info_id(litellm_metadata)? {
        return Ok(build_container_id(
            custom_llm_provider,
            Some(model_id.as_str()),
            container_id,
        ));
    }
    match target_model_names(extra_body)? {
        TargetModelNames::Absent => Ok(container_id.to_owned()),
        TargetModelNames::Present(target_model_id) => Ok(build_container_id(
            custom_llm_provider,
            target_model_id.as_deref(),
            container_id,
        )),
    }
}

// LiteLLM reads target_model_names only when model_info.id yielded nothing, so a wrong-typed value there never fails the call
enum TargetModelNames {
    Absent,
    Present(Option<String>),
}

fn model_info_id(
    litellm_metadata: Option<&IndexMap<String, JsonValue>>,
) -> Result<Option<String>, ContainerFieldTypeError> {
    let Some(metadata) = litellm_metadata else {
        return Ok(None);
    };
    let Some(model_info) = metadata.get("model_info") else {
        return Ok(None);
    };
    match model_info {
        JsonValue::Null => Ok(None),
        JsonValue::Object(fields) => match fields.get("id") {
            None | Some(JsonValue::Null) => Ok(None),
            Some(JsonValue::Text(id)) => Ok(Some(id.clone())),
            Some(
                JsonValue::Bool(_)
                | JsonValue::Number(_)
                | JsonValue::Array(_)
                | JsonValue::Object(_),
            ) => Err(ContainerFieldTypeError {
                field: "litellm_metadata.model_info.id",
            }),
        },
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) | JsonValue::Array(_) => {
            Err(ContainerFieldTypeError {
                field: "litellm_metadata.model_info",
            })
        }
    }
}

// A present but empty list encodes with no model, which is why Present holds an Option
fn target_model_names(
    extra_body: Option<&IndexMap<String, JsonValue>>,
) -> Result<TargetModelNames, ContainerFieldTypeError> {
    let Some(extra_body) = extra_body else {
        return Ok(TargetModelNames::Absent);
    };
    match extra_body.get("target_model_names") {
        None | Some(JsonValue::Null) => Ok(TargetModelNames::Absent),
        Some(JsonValue::Text(names)) => {
            let first = names
                .split(',')
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned();
            Ok(TargetModelNames::Present(Some(first)))
        }
        Some(JsonValue::Array(models)) => match models.first() {
            None => Ok(TargetModelNames::Present(None)),
            Some(JsonValue::Text(model)) => {
                Ok(TargetModelNames::Present(Some(model.trim().to_owned())))
            }
            Some(
                JsonValue::Null
                | JsonValue::Bool(_)
                | JsonValue::Number(_)
                | JsonValue::Array(_)
                | JsonValue::Object(_),
            ) => Err(ContainerFieldTypeError {
                field: "extra_body.target_model_names",
            }),
        },
        Some(JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Object(_)) => {
            Err(ContainerFieldTypeError {
                field: "extra_body.target_model_names",
            })
        }
    }
}
