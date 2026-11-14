use std::collections::BTreeMap;

use betterlite_config::sections::studio_identity::Settings;
use betterlite_openai::api::vector_store_records::records::vector::{
    BaseVectorStoreAuthCredentials, VectorStoreIndexEndpoints,
};
use secrecy::SecretString;

use super::RequestParams;
use crate::studio_files::resolve_key;
use crate::studio_identity::Headers;

const READ_ENDPOINTS: [(&str, &str); 1] = [("POST", "/models/{model}:generateContent")];
const WRITE_ENDPOINTS: [(&str, &str); 1] = [("POST", "/fileSearchStores")];
const SUPPORTED_OPENAI_PARAMS: [&str; 2] = ["max_num_results", "filters"];

pub struct Capabilities {
    pub endpoints: VectorStoreIndexEndpoints,
    pub credentials: BaseVectorStoreAuthCredentials,
    pub supported_params: [&'static str; 2],
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

// get_auth_credentials returns an empty dict, so the key rides in x-goog-api-key instead.
#[must_use]
pub fn validate_environment(
    caller: &BTreeMap<String, String>,
    params: Option<&RequestParams>,
    settings: &Settings,
) -> Headers {
    let mut headers: Headers = caller
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.as_str())))
        .collect();
    headers
        .entry("Content-Type".to_owned())
        .or_insert_with(|| SecretString::from("application/json"));
    if let Some(params) = params {
        if let Some(key) = resolve_key(&[params.api_key.as_ref()], settings) {
            headers.insert("x-goog-api-key".to_owned(), key);
        }
    }
    headers
}
