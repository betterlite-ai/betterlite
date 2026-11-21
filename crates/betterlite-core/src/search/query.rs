use betterlite_models::decisions::Json;
use betterlite_openai::search::provider::SearchQuery;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum QueryError {
    #[error("query must be a string or list of strings, got {found}")]
    Kind { found: &'static str },
    #[error("All items in query list must be strings")]
    Item,
}

pub(super) fn search_query(query: &Json) -> Result<SearchQuery, QueryError> {
    match query {
        Json::Text(text) => Ok(SearchQuery::Single(text.clone())),
        Json::List(items) => items
            .iter()
            .map(|item| match item {
                Json::Text(text) => Ok(text.clone()),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    Err(QueryError::Item)
                }
            })
            .collect::<Result<Vec<_>, _>>()
            .map(SearchQuery::Multiple),
        Json::Null => Err(QueryError::Kind { found: "null" }),
        Json::Bool(_) => Err(QueryError::Kind { found: "a boolean" }),
        Json::Number(_) => Err(QueryError::Kind { found: "a number" }),
        Json::Object(_) => Err(QueryError::Kind { found: "an object" }),
    }
}
