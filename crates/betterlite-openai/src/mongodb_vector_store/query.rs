use crate::vector_stores::store::Query;

use super::error::MongoDbError;

const MAX_QUERY_CHARACTERS: usize = 32_000;
const QUERY_EMPTY: &str = "query must not be empty";

pub(super) fn query_text(query: &Query) -> Result<String, MongoDbError> {
    let text = match query {
        Query::One(text) => text.clone(),
        Query::Many(parts) => parts.join(" "),
    };
    if text.trim().is_empty() {
        return Err(MongoDbError::BadRequest {
            message: QUERY_EMPTY.to_owned(),
        });
    }
    let length = text.chars().count();
    if length > MAX_QUERY_CHARACTERS {
        return Err(MongoDbError::BadRequest {
            message: format!(
                "query must be at most {MAX_QUERY_CHARACTERS} characters, got {length}"
            ),
        });
    }
    Ok(text)
}
