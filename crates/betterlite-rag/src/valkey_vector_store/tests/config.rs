use std::collections::BTreeMap;

use betterlite_provider::api::vector_store_contracts::config::{
    SearchQuery, SearchRequest, VectorStoreConfig as _, VectorStoreLogging, VectorStoreResponse,
};

use super::fakes::empty_params;
use crate::valkey_vector_store::config::ValkeyVectorStoreConfig;
use crate::valkey_vector_store::error::ValkeyError;

const SEARCH_ONLY: &str =
    "Valkey vector store is search-only; create indexes with FT.CREATE directly";

fn response() -> VectorStoreResponse {
    VectorStoreResponse {
        status_code: 200,
        headers: BTreeMap::new(),
        body: Vec::new(),
    }
}

#[test]
fn create_request_fails_with_the_search_only_message() {
    let config = ValkeyVectorStoreConfig;
    let error = config
        .transform_create_request(&empty_params(), "https://api.example.com")
        .expect_err("the create request");
    assert!(matches!(error, ValkeyError::SearchOnly));
    assert_eq!(error.to_string(), SEARCH_ONLY);
}

#[test]
fn create_response_fails_with_the_search_only_message() {
    let config = ValkeyVectorStoreConfig;
    let error = config
        .transform_create_response(&response())
        .expect_err("the create response");
    assert!(matches!(error, ValkeyError::SearchOnly));
    assert_eq!(error.to_string(), SEARCH_ONLY);
}

#[test]
fn complete_url_keeps_an_absent_base_empty_and_create_still_fails_search_only() {
    let config = ValkeyVectorStoreConfig;
    let params = empty_params();
    let base = config
        .complete_url(None, &params)
        .expect("the complete url without a base");
    assert_eq!(base, "");
    let error = config
        .transform_create_request(&params, &base)
        .expect_err("the create request");
    assert!(matches!(error, ValkeyError::SearchOnly));
    assert_eq!(error.to_string(), SEARCH_ONLY);
    assert_eq!(
        config
            .complete_url(Some("https://api.example.com"), &params)
            .expect("the complete url with a base"),
        "https://api.example.com"
    );
}

#[test]
fn search_transforms_refuse_because_the_store_runs_ft_search_itself() {
    let config = ValkeyVectorStoreConfig;
    let params = empty_params();
    let query = SearchQuery::Text("find me".to_owned());
    let mut logging = VectorStoreLogging::default();
    let request = SearchRequest {
        vector_store_id: "vs_index",
        query: &query,
        optional_params: &params,
        api_base: "",
        logging: &mut logging,
        litellm_params: &params,
        extra_body: None,
    };
    let error = config
        .transform_search_request(request)
        .expect_err("the search request");
    assert!(matches!(error, ValkeyError::DirectSearch));
}
