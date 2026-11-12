use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

#[derive(Debug, thiserror::Error)]
pub enum DeploymentReadError {
    #[error("the deployment is not an object")]
    NotObject,
    #[error("the deployment's {field} is missing or not an object")]
    FieldNotObject { field: &'static str },
    #[error("litellm_params.model is missing or not a string")]
    MissingModel,
    #[error("{field} is not a string")]
    NotText { field: &'static str },
}

#[derive(Clone, Copy, Debug)]
pub struct DeploymentTarget<'maps> {
    pub model_id: Option<&'maps str>,
    pub model: &'maps str,
    pub custom_llm_provider: Option<&'maps str>,
}

#[derive(Clone, Copy, Debug)]
pub struct Deployment<'maps> {
    pub litellm_params: &'maps JsonObject,
    pub model_info: &'maps JsonObject,
    pub target: DeploymentTarget<'maps>,
}

fn optional_text<'map>(
    map: &'map JsonObject,
    field: &'static str,
) -> Result<Option<&'map str>, DeploymentReadError> {
    match map.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(_) => Err(DeploymentReadError::NotText { field }),
    }
}

fn object_field<'map>(
    fields: &'map JsonObject,
    field: &'static str,
) -> Result<&'map JsonObject, DeploymentReadError> {
    match fields.get(field) {
        Some(Json::Object(found)) => Ok(found),
        _ => Err(DeploymentReadError::FieldNotObject { field }),
    }
}

impl<'maps> Deployment<'maps> {
    pub fn read(
        litellm_params: &'maps JsonObject,
        model_info: &'maps JsonObject,
    ) -> Result<Self, DeploymentReadError> {
        let Some(Json::Text(model)) = litellm_params.get("model") else {
            return Err(DeploymentReadError::MissingModel);
        };
        Ok(Self {
            litellm_params,
            model_info,
            target: DeploymentTarget {
                model_id: optional_text(model_info, "id")?,
                model,
                custom_llm_provider: optional_text(litellm_params, "custom_llm_provider")?,
            },
        })
    }

    pub fn from_entry(entry: &'maps Json) -> Result<Self, DeploymentReadError> {
        let Json::Object(fields) = entry else {
            return Err(DeploymentReadError::NotObject);
        };
        Self::read(
            object_field(fields, "litellm_params")?,
            object_field(fields, "model_info")?,
        )
    }
}
