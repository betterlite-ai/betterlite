use std::collections::BTreeMap;

use betterlite_bedrock::aws_endpoints::region::non_llm_region_name;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::basic::VectorStoreOpenAiParam;
use betterlite_openai::api::vector_store_records::records::vector::{
    BaseVectorStoreAuthCredentials, VectorStoreIndexEndpoints,
};
use betterlite_provider::api::vector_store_contracts::config::{Headers, Params};

use super::error::S3VectorsSearchError;

pub const DEFAULT_QUERY_EMBEDDING_MODEL: &str = "text-embedding-3-small";
const MAX_NUM_RESULTS: &str = "max_num_results";
const MAX_RESULTS: &str = "maxResults";

#[must_use]
pub const fn auth_credentials() -> BaseVectorStoreAuthCredentials {
    BaseVectorStoreAuthCredentials {
        headers: None,
        query_params: None,
        extra: BTreeMap::new(),
    }
}

#[must_use]
pub fn endpoints_by_type() -> VectorStoreIndexEndpoints {
    VectorStoreIndexEndpoints {
        read: Some(vec![("POST".to_owned(), "/QueryVectors".to_owned())]),
        write: Some(Vec::new()),
        extra: BTreeMap::new(),
    }
}

pub const SUPPORTED_OPENAI_PARAMS: [VectorStoreOpenAiParam; 1] =
    [VectorStoreOpenAiParam::MaxNumResults];

// the query request reads max_num_results from the search params, so maxResults is stored and never sent
#[must_use]
pub fn map_openai_params(non_default_params: &Params, mut optional_params: Params) -> Params {
    if let Some(value) = non_default_params.get(MAX_NUM_RESULTS) {
        optional_params.insert(MAX_RESULTS.to_owned(), value.clone());
    }
    optional_params
}

#[must_use]
pub fn validate_environment(mut headers: Headers) -> Headers {
    headers
        .entry("Content-Type".to_owned())
        .or_insert_with(|| "application/json".to_owned());
    headers
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RegionEnvironment<'env> {
    pub region_name: Option<&'env str>,
    pub region: Option<&'env str>,
}

// api_base is ignored: only the region picks the host
pub fn complete_url(
    litellm_params: &Params,
    environment: RegionEnvironment<'_>,
) -> Result<String, S3VectorsSearchError> {
    let explicit = text_param(litellm_params, "aws_region_name")?;
    let region = non_llm_region_name(explicit, environment.region_name, environment.region)?;
    Ok(format!("https://s3vectors.{region}.api.aws"))
}

pub fn query_embedding_model(litellm_params: &Params) -> Result<String, S3VectorsSearchError> {
    for field in ["litellm_embedding_model", "embedding_model"] {
        if let Some(model) = text_param(litellm_params, field)?.filter(|model| !model.is_empty()) {
            return Ok(model.to_owned());
        }
    }
    Ok(DEFAULT_QUERY_EMBEDDING_MODEL.to_owned())
}

pub(super) fn text_param<'params>(
    params: &'params Params,
    field: &'static str,
) -> Result<Option<&'params str>, S3VectorsSearchError> {
    match params.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(_) => Err(S3VectorsSearchError::ParamNotText { field }),
    }
}
