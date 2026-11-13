use super::custom_fields::CUSTOM_PRICING_FIELDS;
use super::error::DeploymentIdError;
use crate::decisions::Json;
use crate::model_registration::JsonObject;

const MODEL_INFO_DEFAULTS: [&str; 3] = ["mode", "supports_prompt_caching", "max_tokens"];
const METADATA_BUCKETS: [&str; 2] = ["litellm_metadata", "metadata"];

#[must_use]
pub fn build_custom_pricing_entry(
    custom_llm_provider: &str,
    kwargs: &JsonObject,
    model_info: Option<&JsonObject>,
) -> JsonObject {
    let mut entry = JsonObject::new();
    entry.insert(
        "litellm_provider".to_owned(),
        Json::Text(custom_llm_provider.to_owned()),
    );
    for field in CUSTOM_PRICING_FIELDS {
        match kwargs.get(*field) {
            None | Some(Json::Null) => {}
            Some(value) => {
                entry.insert((*field).to_owned(), value.clone());
            }
        }
    }
    for field in MODEL_INFO_DEFAULTS {
        match model_info.and_then(|info| info.get(field)) {
            None | Some(Json::Null) => {}
            Some(value) => {
                entry
                    .entry(field.to_owned())
                    .or_insert_with(|| value.clone());
            }
        }
    }
    entry
}

pub fn router_deployment_id(kwargs: &JsonObject) -> Result<Option<String>, DeploymentIdError> {
    for bucket in METADATA_BUCKETS {
        let Some(Json::Object(metadata)) = kwargs.get(bucket) else {
            continue;
        };
        let Some(Json::Object(deployment_model_info)) = metadata.get("model_info") else {
            continue;
        };
        match deployment_model_info.get("id") {
            None | Some(Json::Null) => {}
            Some(Json::Text(id)) => return Ok(Some(id.clone())),
            Some(Json::Number(id)) => return Ok(Some(id.to_string())),
            Some(Json::Bool(_) | Json::List(_) | Json::Object(_)) => {
                return Err(DeploymentIdError::NotScalar { bucket });
            }
        }
    }
    Ok(None)
}
