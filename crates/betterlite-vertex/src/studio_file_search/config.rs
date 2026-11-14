use std::collections::BTreeMap;

use betterlite_config::sections::studio_identity::Settings;
use betterlite_openai::api::vector_store_records::records::vector::{
    BaseVectorStoreAuthCredentials, VectorStoreIndexEndpoints,
};
use secrecy::SecretString;

use super::RequestParams;
use crate::studio_files::resolve_key;
use crate::studio_identity::Headers;

pub struct Capabilities {
    pub endpoints: VectorStoreIndexEndpoints,
    pub credentials: BaseVectorStoreAuthCredentials,
    pub supported_params: [&'static str; 2],
}

#[must_use]
pub fn capabilities() -> Capabilities {
    Capabilities {
        endpoints: VectorStoreIndexEndpoints {
            read: Some(vec![(
                "POST".to_owned(),
                "/models/{model}:generateContent".to_owned(),
            )]),
            write: Some(vec![("POST".to_owned(), "/fileSearchStores".to_owned())]),
            extra: BTreeMap::new(),
        },
        credentials: BaseVectorStoreAuthCredentials {
            headers: None,
            query_params: None,
            extra: BTreeMap::new(),
        },
        supported_params: ["max_num_results", "filters"],
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
    if let Some(params) = params
        && let Some(key) = resolve_key(&[params.api_key.as_ref()], settings)
    {
        headers.insert("x-goog-api-key".to_owned(), key);
    }
    headers
}
