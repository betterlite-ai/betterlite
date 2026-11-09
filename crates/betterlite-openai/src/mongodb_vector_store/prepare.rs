use betterlite_models::decisions::Json;
use serde::Serialize;

use super::Params;
use super::error::MongoDbError;
use super::params::typed_params;
use super::query::query_text;
use crate::vector_stores::store::{Query, SearchParams};

const DEFAULT_EMBEDDING_FIELD: &str = "embedding";
const DEFAULT_TEXT_FIELD: &str = "text";
const DEFAULT_MAX_NUM_RESULTS: i64 = 10;
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const MIN_MAX_NUM_RESULTS: i64 = 1;
const MAX_MAX_NUM_RESULTS: i64 = 50;
const NUM_CANDIDATES_MULTIPLIER: i64 = 10;
const MIN_NUM_CANDIDATES: i64 = 100;
const MAX_NUM_CANDIDATES: i64 = 10_000;
const KNOWN_MONGODB_PARAMS: [&str; 5] = [
    "mongodb_collection",
    "mongodb_database",
    "mongodb_embedding_field",
    "mongodb_num_candidates",
    "mongodb_text_field",
];
const CONNECTION_STRING: &str = "MongoDB vector stores now use the BETA sidecar. Move mongodb_connection_string to MONGODB_CONNECTION_STRING in the sidecar, remove it from LiteLLM, and configure api_base and api_key.";
const EXTRA_BODY: &str = "MongoDB vector store does not support extra_body overrides.";
const MISSING_DATABASE: &str = "mongodb_database is required in litellm_params for the MongoDB vector store. Example: mongodb_database: sample_mflix";
const MISSING_COLLECTION: &str = "mongodb_collection is required in litellm_params for the MongoDB vector store. Example: mongodb_collection: embedded_movies";
const MISSING_EMBEDDING_MODEL: &str = "litellm_embedding_model is required in litellm_params for the MongoDB vector store. It must be the same model that produced the vectors stored in";
const TIMEOUT: &str = "MongoDB search timeout must be a positive finite number.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreparedSearch {
    pub query_text: String,
    pub embedding_model: String,
    pub embedding_config: Params,
    pub database: String,
    pub collection: String,
    pub text_field: String,
    pub embedding_field: String,
    pub num_candidates: i64,
    pub max_num_results: i64,
    pub timeout_ms: u64,
}

pub fn prepare_search(
    litellm_params: &Params,
    options: &SearchParams,
    extra_body: Option<&Params>,
    query: &Query,
) -> Result<PreparedSearch, MongoDbError> {
    reject_unknown_params(litellm_params)?;
    if extra_body.is_some_and(|body| !body.is_empty()) {
        return Err(MongoDbError::BadRequest {
            message: EXTRA_BODY.to_owned(),
        });
    }
    reject_unsupported(options)?;
    let typed = typed_params(litellm_params)?;
    let database = require(typed.database, MISSING_DATABASE)?;
    let collection = require(typed.collection, MISSING_COLLECTION)?;
    let embedding_field = default_field(typed.embedding_field, DEFAULT_EMBEDDING_FIELD);
    let Some(embedding_model) = typed.embedding_model.filter(|model| !model.is_empty()) else {
        return Err(MongoDbError::BadRequest {
            message: format!(
                "{MISSING_EMBEDDING_MODEL} '{collection}.{embedding_field}', or search results will be meaningless. Example: litellm_embedding_model: openai/text-embedding-3-small"
            ),
        });
    };
    let text_field = default_field(typed.text_field, DEFAULT_TEXT_FIELD);
    let max_num_results = limit(options)?;
    let num_candidates = num_candidates(max_num_results, typed.num_candidates)?;
    let timeout_ms = timeout_ms(litellm_params)?;
    Ok(PreparedSearch {
        query_text: query_text(query)?,
        embedding_model,
        embedding_config: typed.embedding_config.unwrap_or_default(),
        database,
        collection,
        text_field,
        embedding_field,
        num_candidates,
        max_num_results,
        timeout_ms,
    })
}

pub(crate) fn reject_unknown_params(params: &Params) -> Result<(), MongoDbError> {
    if params
        .get("mongodb_connection_string")
        .is_some_and(|value| !matches!(value, Json::Null))
    {
        return Err(MongoDbError::BadRequest {
            message: CONNECTION_STRING.to_owned(),
        });
    }
    let mut unknown: Vec<&str> = params
        .keys()
        .filter(|key| key.starts_with("mongodb_") && !KNOWN_MONGODB_PARAMS.contains(&key.as_str()))
        .map(String::as_str)
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(MongoDbError::BadRequest {
        message: format!(
            "Unrecognised MongoDB vector store parameter(s): {}. Supported: {}.",
            unknown.join(", "),
            KNOWN_MONGODB_PARAMS.join(", ")
        ),
    })
}

fn reject_unsupported(options: &SearchParams) -> Result<(), MongoDbError> {
    let unsupported = if options.filters.is_some() {
        "filters"
    } else if options.ranking_options.is_some() {
        "ranking_options"
    } else if options.rewrite_query.is_some() {
        "rewrite_query"
    } else {
        return Ok(());
    };
    Err(MongoDbError::BadRequest {
        message: format!("MongoDB vector store does not support the {unsupported} parameter."),
    })
}

fn require(value: Option<String>, message: &str) -> Result<String, MongoDbError> {
    value
        .filter(|field| !field.is_empty())
        .ok_or_else(|| MongoDbError::BadRequest {
            message: message.to_owned(),
        })
}

fn default_field(value: Option<String>, default: &str) -> String {
    value
        .filter(|field| !field.is_empty())
        .unwrap_or_else(|| default.to_owned())
}

fn limit(options: &SearchParams) -> Result<i64, MongoDbError> {
    let Some(requested) = options.max_num_results else {
        return Ok(DEFAULT_MAX_NUM_RESULTS);
    };
    if !(MIN_MAX_NUM_RESULTS..=MAX_MAX_NUM_RESULTS).contains(&requested) {
        return Err(MongoDbError::BadRequest {
            message: format!(
                "max_num_results must be between {MIN_MAX_NUM_RESULTS} and {MAX_MAX_NUM_RESULTS}, got {requested}"
            ),
        });
    }
    Ok(requested)
}

fn num_candidates(limit: i64, configured: Option<i64>) -> Result<i64, MongoDbError> {
    let Some(chosen) = configured else {
        return Ok(limit
            .saturating_mul(NUM_CANDIDATES_MULTIPLIER)
            .clamp(MIN_NUM_CANDIDATES, MAX_NUM_CANDIDATES));
    };
    if !(limit..=MAX_NUM_CANDIDATES).contains(&chosen) {
        return Err(MongoDbError::BadRequest {
            message: format!(
                "mongodb_num_candidates must be between max_num_results ({limit}) and {MAX_NUM_CANDIDATES}, got {chosen}"
            ),
        });
    }
    Ok(chosen)
}

fn timeout_ms(params: &Params) -> Result<u64, MongoDbError> {
    let Some(seconds) = read_timeout(params)? else {
        return Ok(DEFAULT_TIMEOUT_MS);
    };
    if !seconds.is_finite() || seconds <= 0.0_f64 {
        return Err(MongoDbError::BadRequest {
            message: TIMEOUT.to_owned(),
        });
    }
    // a fused multiply-add avoids clippy's float-arithmetic-operator restriction and rounds like Python's product
    let millis = seconds.mul_add(1000.0_f64, 0.0_f64).trunc();
    if !millis.is_finite() {
        return Err(MongoDbError::BadRequest {
            message: TIMEOUT.to_owned(),
        });
    }
    if millis < 1.0_f64 {
        return Ok(1);
    }
    // {:.0} prints the truncated integer, since a bare Display can spell a large value as an exponent
    format!("{millis:.0}")
        .parse::<u64>()
        .map_err(|_error| MongoDbError::BadRequest {
            message: TIMEOUT.to_owned(),
        })
}

// an untagged enum reads a lexeme that does not round-trip through f64 as a map, so read the Json number directly
fn read_timeout(params: &Params) -> Result<Option<f64>, MongoDbError> {
    let Some(value) = params.get("timeout") else {
        return Ok(None);
    };
    let read = match value {
        Json::Null => None,
        Json::Number(number) => Some(number),
        Json::Object(fields) => match fields.get("read") {
            None | Some(Json::Null) => None,
            Some(Json::Number(number)) => Some(number),
            Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
                return Err(MongoDbError::BadRequest {
                    message: TIMEOUT.to_owned(),
                });
            }
        },
        Json::Bool(_) | Json::Text(_) | Json::List(_) => {
            return Err(MongoDbError::BadRequest {
                message: TIMEOUT.to_owned(),
            });
        }
    };
    let Some(number) = read else {
        return Ok(None);
    };
    match number.as_f64() {
        Some(seconds) if seconds.is_finite() => Ok(Some(seconds)),
        _ => Err(MongoDbError::BadRequest {
            message: TIMEOUT.to_owned(),
        }),
    }
}
