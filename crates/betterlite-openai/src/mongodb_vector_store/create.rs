use crate::api::vector_store_records::records::vector::VectorStoreCreateResponse;

use super::error::MongoDbError;
use super::search::SearchRequest;

const SEARCH_ONLY: &str = "MongoDB vector store is search-only. Create the collection and its MongoDB Vector Search index in MongoDB directly, then register it here by index name.";

pub fn validate_create_vector_store() -> Result<(), MongoDbError> {
    Err(MongoDbError::BadRequest {
        message: SEARCH_ONLY.to_owned(),
    })
}

pub fn create_request() -> Result<SearchRequest, MongoDbError> {
    Err(MongoDbError::BadRequest {
        message: SEARCH_ONLY.to_owned(),
    })
}

pub fn create_response() -> Result<VectorStoreCreateResponse, MongoDbError> {
    Err(MongoDbError::BadRequest {
        message: SEARCH_ONLY.to_owned(),
    })
}
