use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::vector_store_records::records::vector::{
    VectorStoreCreateResponse, VectorStoreIndexEndpoints,
};
use crate::vector_stores::{Headers, Request};
use error::{MilvusError, ResponseHeaders, UnsupportedCreate};

pub mod decode;
pub mod error;
pub mod search;
mod tests;

pub type OptionalParams = BTreeMap<String, Json>;

// LiteLLM's MILVUS_OPTIONAL_PARAMS: the search parameters its mapper copies from the caller
const MILVUS_OPTIONAL_PARAMS: [&str; 8] = [
    "annsField",
    "limit",
    "filter",
    "offset",
    "groupingField",
    "outputFields",
    "searchParams",
    "consistencyLevel",
];

#[derive(Debug, Clone, Default)]
pub struct MilvusEnv {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct MilvusParams {
    pub api_key: Option<SecretString>,
    pub milvus_db_name: Option<String>,
    pub milvus_partition_names: Option<Vec<String>>,
    pub milvus_text_field: Option<String>,
    pub litellm_embedding_model: Option<String>,
}

pub fn validate_environment(
    mut headers: Headers,
    litellm_params: Option<&MilvusParams>,
    env: &MilvusEnv,
) -> Result<Headers, MilvusError> {
    // LiteLLM reads the environment key only inside the Some(litellm_params) branch
    let params = litellm_params.ok_or(MilvusError::MissingApiKey)?;
    let explicit = params
        .api_key
        .as_ref()
        .filter(|key| !key.expose_secret().is_empty());
    let api_key = explicit
        .or_else(|| {
            env.api_key
                .as_ref()
                .filter(|key| !key.expose_secret().is_empty())
        })
        .ok_or(MilvusError::MissingApiKey)?;
    headers.insert(
        "Authorization".to_owned(),
        SecretString::from(format!("Bearer {}", api_key.expose_secret())),
    );
    Ok(headers)
}

pub fn get_auth_credentials(litellm_params: &MilvusParams) -> Result<Headers, MilvusError> {
    let api_key = litellm_params
        .api_key
        .as_ref()
        .filter(|key| !key.expose_secret().is_empty())
        .ok_or(MilvusError::MissingApiKey)?;
    Ok(BTreeMap::from([(
        "Authorization".to_owned(),
        SecretString::from(format!("Bearer {}", api_key.expose_secret())),
    )]))
}

pub fn get_complete_url(api_base: Option<&str>, env: &MilvusEnv) -> Result<String, MilvusError> {
    let base = api_base
        .filter(|base| !base.is_empty())
        .or_else(|| env.api_base.as_deref().filter(|base| !base.is_empty()))
        .ok_or(MilvusError::MissingApiBase)?;
    Ok(base.trim_end_matches('/').to_owned())
}

#[must_use]
pub fn endpoints() -> VectorStoreIndexEndpoints {
    let read = [
        ("POST", "/v2/vectordb/entities/search"),
        ("POST", "/v2/vectordb/entities/get"),
        ("POST", "/v2/vectordb/entities/query"),
    ];
    let write = [
        ("POST", "/v2/vectordb/entities/upsert"),
        ("POST", "/v2/vectordb/entities/insert"),
    ];
    let owned = |routes: &[(&str, &str)]| {
        routes
            .iter()
            .map(|(method, path)| ((*method).to_owned(), (*path).to_owned()))
            .collect()
    };
    VectorStoreIndexEndpoints {
        read: Some(owned(&read)),
        write: Some(owned(&write)),
        extra: BTreeMap::new(),
    }
}

#[must_use]
pub fn map_openai_params(
    non_default_params: &OptionalParams,
    mut optional_params: OptionalParams,
    _drop_params: bool,
) -> OptionalParams {
    for (param, value) in non_default_params {
        if MILVUS_OPTIONAL_PARAMS.contains(&param.as_str()) {
            optional_params.insert(param.clone(), value.clone());
        }
    }
    optional_params
}

pub fn create_request(
    _optional_params: &OptionalParams,
    api_base: &str,
) -> Result<Request, MilvusError> {
    Err(MilvusError::CreateUnsupported(UnsupportedCreate::Request {
        api_base: api_base.to_owned(),
    }))
}

pub fn create_response(
    status: u16,
    headers: &ResponseHeaders,
) -> Result<VectorStoreCreateResponse, MilvusError> {
    Err(MilvusError::CreateUnsupported(
        UnsupportedCreate::Response {
            status,
            headers: headers.clone(),
        },
    ))
}
