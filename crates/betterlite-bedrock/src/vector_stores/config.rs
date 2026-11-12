use std::collections::BTreeMap;

use betterlite_openai::api::vector_store_records::records::vector::{
    BaseVectorStoreAuthCredentials, VectorStoreIndexEndpoints,
};

const READ_ENDPOINTS: [(&str, &str); 1] =
    [("POST", "/knowledgebases/{knowledge_base_id}/retrieve")];
const WRITE_ENDPOINTS: [(&str, &str); 0] = [];
const SUPPORTED_OPENAI_PARAMS: [&str; 3] = ["filters", "max_num_results", "ranking_options"];

pub struct Capabilities {
    pub endpoints: VectorStoreIndexEndpoints,
    pub credentials: BaseVectorStoreAuthCredentials,
    pub supported_params: [&'static str; 3],
}

fn endpoint_pairs(endpoints: &[(&str, &str)]) -> Vec<(String, String)> {
    endpoints
        .iter()
        .map(|(method, path)| ((*method).to_owned(), (*path).to_owned()))
        .collect()
}

#[must_use]
pub fn capabilities() -> Capabilities {
    Capabilities {
        endpoints: VectorStoreIndexEndpoints {
            read: Some(endpoint_pairs(&READ_ENDPOINTS)),
            write: Some(endpoint_pairs(&WRITE_ENDPOINTS)),
            extra: BTreeMap::new(),
        },
        credentials: BaseVectorStoreAuthCredentials {
            headers: None,
            query_params: None,
            extra: BTreeMap::new(),
        },
        supported_params: SUPPORTED_OPENAI_PARAMS,
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
