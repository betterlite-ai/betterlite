use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RouterWeightModelInfo {
    pub team_id: Option<String>,
    pub db_model: Option<bool>,
    pub team_public_model_name: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum RouterWeightModelInfoError {
    #[error("model_info must be a JSON object")]
    NotAnObject,
    #[error("model_info is not valid JSON")]
    Json(#[source] serde_json::Error),
    #[error("model_info.{field} has the wrong type")]
    FieldType { field: &'static str },
}

impl RouterWeightModelInfo {
    pub(crate) fn decode(value: &Json) -> Result<Self, RouterWeightModelInfoError> {
        match value {
            Json::Text(text) => {
                let parsed: Json =
                    serde_json::from_str(text).map_err(RouterWeightModelInfoError::Json)?;
                let Json::Object(fields) = parsed else {
                    return Err(RouterWeightModelInfoError::NotAnObject);
                };
                Self::from_fields(&fields)
            }
            Json::Object(fields) => Self::from_fields(fields),
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

    fn from_fields(fields: &JsonObject) -> Result<Self, RouterWeightModelInfoError> {
        Ok(Self {
            team_id: text_field(fields.get("team_id"), "team_id")?,
            db_model: bool_field(fields.get("db_model"))?,
            team_public_model_name: text_field(
                fields.get("team_public_model_name"),
                "team_public_model_name",
            )?,
        })
    }
}

fn text_field(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<String>, RouterWeightModelInfoError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(RouterWeightModelInfoError::FieldType { field })
        }
    }
}

fn bool_field(value: Option<&Json>) -> Result<Option<bool>, RouterWeightModelInfoError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(flag)) => Ok(Some(*flag)),
        Some(Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(RouterWeightModelInfoError::FieldType { field: "db_model" })
        }
    }
}
