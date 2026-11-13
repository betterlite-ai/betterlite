use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("the {record} field `{path}` has the wrong type")]
    Field {
        record: &'static str,
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the {record} record could not be encoded for field checking")]
    Encode {
        record: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

pub fn decode<T>(record: &'static str, fields: IndexMap<String, Json>) -> Result<T, DecodeError>
where
    T: DeserializeOwned,
{
    let value = serde_json::to_value(Json::Object(fields))
        .map_err(|source| DecodeError::Encode { record, source })?;
    serde_path_to_error::deserialize(value).map_err(|error| DecodeError::Field {
        record,
        path: error.path().to_string(),
        source: error.into_inner(),
    })
}

mod tests;
