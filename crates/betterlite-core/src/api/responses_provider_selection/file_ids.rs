use super::ParameterMap;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_prompts::file_ids::limits::FileIdsError;
use betterlite_prompts::file_ids::mapping::FileIdMapping;
use betterlite_prompts::file_ids::responses::{ResponsesInput, update_responses_file_ids};
use betterlite_prompts::file_ids::tools::{ResponsesTool, update_tools_with_file_ids};
use serde::Deserialize as _;
use thiserror::Error;
#[derive(Debug, Error)]
pub enum FileMappingError {
    #[error(transparent)]
    FileIds(#[from] FileIdsError),
    #[error("the mapped value could not be encoded: {message}")]
    Encode { message: String },
}
pub struct FileMappingOutcome {
    pub input: ResponsesInput,
    pub tools: Option<Vec<ResponsesTool>>,
}
pub fn apply_managed_file_id_mapping(
    input: &ResponsesInput,
    tools: Option<&[ResponsesTool]>,
    model_info: Option<&JsonValue>,
    mapping: Option<&FileIdMapping>,
    local_vars: &mut ParameterMap,
) -> Result<FileMappingOutcome, FileMappingError> {
    let model_id = model_info_id(model_info);
    let fallback = FileIdMapping::default();
    let resolved_mapping = mapping.unwrap_or(&fallback);
    let updated_input = update_responses_file_ids(input, model_id.as_deref(), resolved_mapping)?;
    local_vars.insert(String::from("input"), json_value(&updated_input)?);
    let Some(present) = tools.filter(|present| !present.is_empty()) else {
        return Ok(FileMappingOutcome {
            input: updated_input,
            tools: tools.map(<[ResponsesTool]>::to_vec),
        });
    };
    let updated_tools =
        update_tools_with_file_ids(Some(present), model_id.as_deref(), resolved_mapping)?
            .unwrap_or_else(|| present.to_vec());
    local_vars.insert(String::from("tools"), json_value(&updated_tools)?);
    Ok(FileMappingOutcome {
        input: updated_input,
        tools: Some(updated_tools),
    })
}
#[must_use]
pub fn model_info_id(model_info: Option<&JsonValue>) -> Option<String> {
    let Some(JsonValue::Object(fields)) = model_info else {
        return None;
    };
    match fields.get("id") {
        Some(JsonValue::Text(id)) => Some(id.clone()),
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        )
        | None => None,
    }
}
fn json_value<T: serde::Serialize>(value: &T) -> Result<JsonValue, FileMappingError> {
    let raw = serde_json::to_value(value).map_err(|source| FileMappingError::Encode {
        message: source.to_string(),
    })?;
    JsonValue::deserialize(raw).map_err(|source| FileMappingError::Encode {
        message: source.to_string(),
    })
}
