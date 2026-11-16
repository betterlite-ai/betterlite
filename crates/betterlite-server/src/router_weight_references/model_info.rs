use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_with::{DefaultOnNull, serde_as};

#[serde_as]
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RouterWeightModelInfo {
    pub team_id: Option<String>,
    #[serde_as(as = "DefaultOnNull")]
    pub db_model: bool,
    #[serde_as(as = "DefaultOnNull")]
    pub team_public_model_name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RouterWeightModelInfoError {
    #[error("model_info must be a JSON object")]
    NotAnObject,
    #[error("model_info is not valid")]
    Json(#[source] serde_json::Error),
}

impl RouterWeightModelInfo {
    pub(crate) fn decode(value: &Json) -> Result<Self, RouterWeightModelInfoError> {
        match value {
            Json::Text(text) => {
                let parsed: Json =
                    serde_json::from_str(text).map_err(RouterWeightModelInfoError::Json)?;
                if !matches!(parsed, Json::Object(_)) {
                    return Err(RouterWeightModelInfoError::NotAnObject);
                }
                Self::from_object(&parsed)
            }
            Json::Object(_) => Self::from_object(value),
            Json::Null | Json::Bool(false) => Ok(Self::default()),
            Json::List(items) if items.is_empty() => Ok(Self::default()),
            Json::Number(number) if number.as_f64().is_some_and(|parsed| parsed == 0.0_f64) => {
                Ok(Self::default())
            }
            Json::Bool(true) | Json::List(_) | Json::Number(_) => {
                Err(RouterWeightModelInfoError::NotAnObject)
            }
        }
    }

    // serde would also read this struct from a JSON array, so callers pass an object
    fn from_object(object: &Json) -> Result<Self, RouterWeightModelInfoError> {
        let text = serde_json::to_string(object).map_err(RouterWeightModelInfoError::Json)?;
        serde_json::from_str(&text).map_err(RouterWeightModelInfoError::Json)
    }
}
