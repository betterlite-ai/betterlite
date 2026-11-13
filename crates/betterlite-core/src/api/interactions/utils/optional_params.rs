use betterlite_context::non_default_params::Params;
use betterlite_context::non_default_params::pre_process::{
    PreProcessRequest, base_pre_process_params,
};
use betterlite_models::controls::json_value::JsonValue;

use super::error::InteractionsUtilsError;

// the optional parameters the Interactions OpenAPI spec declares, in the order LiteLLM lists them
pub const INTERACTION_OPTIONAL_PARAMS: &str = "tools system_instruction generation_config \
    stream store background environment response_modalities response_format response_mime_type \
    previous_interaction_id agent_config";

const ENDPOINT_SPECIFIC_PARAMS: [&str; 3] = ["input", "model", "agent"];

struct ControlFields {
    custom_llm_provider: String,
    special_params: Params,
    additional_drop_params: Vec<String>,
}

pub fn requested_optional_params(params: &Params) -> Result<Params, InteractionsUtilsError> {
    let control = control_fields(params)?;
    let processed = base_pre_process_params(
        &PreProcessRequest {
            passed_params: params,
            special_params: &control.special_params,
            custom_llm_provider: &control.custom_llm_provider,
            additional_drop_params: &control.additional_drop_params,
            provider_config: None,
        },
        INTERACTION_OPTIONAL_PARAMS,
        &ENDPOINT_SPECIFIC_PARAMS,
    );
    Ok(processed.non_default_params)
}

fn control_fields(params: &Params) -> Result<ControlFields, InteractionsUtilsError> {
    // LiteLLM's base filter compares against the provider string, so a call without one behaves as an unnamed provider
    let custom_llm_provider = match params.get("custom_llm_provider") {
        None | Some(JsonValue::Null) => String::new(),
        Some(JsonValue::Text(name)) => name.clone(),
        Some(_) => {
            return Err(InteractionsUtilsError::NotText {
                field: "custom_llm_provider",
            });
        }
    };
    let special_params = match params.get("kwargs") {
        None | Some(JsonValue::Null) => Params::new(),
        Some(JsonValue::Object(fields)) => fields
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        Some(_) => return Err(InteractionsUtilsError::NotAnObject { field: "kwargs" }),
    };
    let additional_drop_params = match params.get("additional_drop_params") {
        None | Some(JsonValue::Null) => Vec::new(),
        Some(JsonValue::Array(items)) => list_of_names(items)?,
        Some(_) => {
            return Err(InteractionsUtilsError::NotList {
                field: "additional_drop_params",
            });
        }
    };
    Ok(ControlFields {
        custom_llm_provider,
        special_params,
        additional_drop_params,
    })
}

fn list_of_names(items: &[JsonValue]) -> Result<Vec<String>, InteractionsUtilsError> {
    let mut names = Vec::with_capacity(items.len());
    for item in items {
        let JsonValue::Text(name) = item else {
            return Err(InteractionsUtilsError::NotText {
                field: "additional_drop_params",
            });
        };
        names.push(name.clone());
    }
    Ok(names)
}
