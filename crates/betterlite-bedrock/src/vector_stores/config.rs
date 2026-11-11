use std::collections::BTreeMap;

use betterlite_openai::api::vector_store_records::records::vector::BaseVectorStoreAuthCredentials;

pub const READ_ENDPOINTS: [(&str, &str); 1] =
    [("POST", "/knowledgebases/{knowledge_base_id}/retrieve")];
pub const WRITE_ENDPOINTS: [(&str, &str); 0] = [];
pub const SUPPORTED_OPENAI_PARAMS: [&str; 3] = ["filters", "max_num_results", "ranking_options"];

#[must_use]
pub const fn auth_credentials() -> BaseVectorStoreAuthCredentials {
    BaseVectorStoreAuthCredentials {
        headers: None,
        query_params: None,
        extra: BTreeMap::new(),
    }
}

#[must_use]
pub fn validate_environment(
    mut headers: BTreeMap<String, Option<String>>,
) -> BTreeMap<String, Option<String>> {
    headers
        .entry("Content-Type".to_owned())
        .or_insert_with(|| Some("application/json".to_owned()));
    headers
}
