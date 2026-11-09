use betterlite_models::decisions::Json;

use super::Params;
use super::error::MongoDbError;

pub(super) const INVALID_CONFIG: &str = "Invalid MongoDB vector-store configuration. Check the database, collection, fields, and candidate count.";

pub(super) struct TypedParams {
    pub(super) embedding_model: Option<String>,
    pub(super) embedding_config: Option<Params>,
    pub(super) database: Option<String>,
    pub(super) collection: Option<String>,
    pub(super) text_field: Option<String>,
    pub(super) embedding_field: Option<String>,
    pub(super) num_candidates: Option<i64>,
}

pub(super) fn typed_params(params: &Params) -> Result<TypedParams, MongoDbError> {
    let embedding_config = match params.get("litellm_embedding_config") {
        None | Some(Json::Null) => None,
        Some(Json::Object(values)) => Some(
            values
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
        Some(_) => {
            return Err(MongoDbError::BadRequest {
                message: INVALID_CONFIG.to_owned(),
            });
        }
    };
    Ok(TypedParams {
        embedding_model: text(params, "litellm_embedding_model")?,
        embedding_config,
        database: text(params, "mongodb_database")?,
        collection: text(params, "mongodb_collection")?,
        text_field: text(params, "mongodb_text_field")?,
        embedding_field: text(params, "mongodb_embedding_field")?,
        num_candidates: integer(params, "mongodb_num_candidates")?,
    })
}

fn text(params: &Params, field: &str) -> Result<Option<String>, MongoDbError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    match value {
        Json::Null => Ok(None),
        Json::Text(text) => Ok(Some(text.clone())),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(MongoDbError::BadRequest {
                message: INVALID_CONFIG.to_owned(),
            })
        }
    }
}

fn integer(params: &Params, field: &str) -> Result<Option<i64>, MongoDbError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    if let Json::Number(number) = value {
        return number
            .as_i64()
            .map(Some)
            .ok_or_else(|| MongoDbError::BadRequest {
                message: INVALID_CONFIG.to_owned(),
            });
    }
    if matches!(value, Json::Null) {
        return Ok(None);
    }
    Err(MongoDbError::BadRequest {
        message: INVALID_CONFIG.to_owned(),
    })
}
