use betterlite_models::decisions::Json;
use betterlite_provider::api::vector_store_contracts::config::{Params, SearchQuery};

use super::error::ValkeyError;

pub const DEFAULT_MAX_NUM_RESULTS: i64 = 10;
pub const MIN_MAX_NUM_RESULTS: i64 = 1;
pub const MAX_MAX_NUM_RESULTS: i64 = 50;
pub const DEFAULT_TEXT_FIELD: &str = "text";
pub const DEFAULT_EMBEDDING_FIELD: &str = "embedding";
pub const DISTANCE_FIELD: &str = "vector_distance";
pub const SEARCH_DIALECT: i64 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValkeyKnnQuery {
    pub expression: String,
    pub return_fields: Vec<String>,
    pub offset: i64,
    pub count: i64,
    pub dialect: i64,
}

pub fn query_text(query: &SearchQuery) -> Result<String, ValkeyError> {
    match query {
        SearchQuery::Text(text) => Ok(text.clone()),
        SearchQuery::Sequence(items) if items.is_empty() => Err(ValkeyError::EmptyQuery),
        SearchQuery::Sequence(items) => Ok(items.join(" ")),
    }
}

pub fn knn_limit(optional_params: &Params) -> Result<i64, ValkeyError> {
    match optional_params.get("max_num_results") {
        None | Some(Json::Null) => Ok(DEFAULT_MAX_NUM_RESULTS),
        Some(Json::Number(number)) => {
            let value = number.as_i64().ok_or(ValkeyError::FieldType {
                field: "max_num_results",
                expected: "an integer",
            })?;
            if (MIN_MAX_NUM_RESULTS..=MAX_MAX_NUM_RESULTS).contains(&value) {
                Ok(value)
            } else {
                Err(ValkeyError::MaxResultsOutOfRange { value })
            }
        }
        Some(_) => Err(ValkeyError::FieldType {
            field: "max_num_results",
            expected: "an integer",
        }),
    }
}

pub fn knn_query(
    optional_params: &Params,
    embedding_field: &str,
    text_field: &str,
) -> Result<ValkeyKnnQuery, ValkeyError> {
    if optional_params
        .get("filters")
        .is_some_and(|filters| !matches!(filters, Json::Null))
    {
        return Err(ValkeyError::FiltersUnsupported);
    }
    let count = knn_limit(optional_params)?;
    Ok(ValkeyKnnQuery {
        expression: format!("*=>[KNN {count} @{embedding_field} $vec AS {DISTANCE_FIELD}]"),
        return_fields: vec![text_field.to_owned(), DISTANCE_FIELD.to_owned()],
        offset: 0,
        count,
        dialect: SEARCH_DIALECT,
    })
}
