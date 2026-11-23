use std::future::ready;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_context::resource_config_selection::vector_stores::VectorStoresConfigTag;
use betterlite_core::api::vector_search_execution::config::{
    EmbeddingEntry, Params, QueryEmbeddingExecutor, SearchConfig, StoreFailure,
};
use betterlite_core::api::vector_search_execution::openai::OpenAiSearchConfig;
use betterlite_core::api::vector_search_execution::service::signing::{
    SignFuture, SignInput, SigningSeam,
};
use betterlite_core::api::vector_store_http_create_read::config::{
    CreateReadConfig, OpenAiCreateReadConfig,
};
use betterlite_core::api::vector_store_search::executor::{
    EmbeddingExecutorSource, SelectedExecutor,
};
use betterlite_core::api::vector_store_search::service::{
    OptionalParamFilters, VectorStoreConfigs,
};
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::embedding::EmbeddingResponse;
use secrecy::SecretString;

// VectorStoreCreateOptionalRequestParams: the only keys a create body may carry
const CREATE_FIELDS: [&str; 5] = [
    "name",
    "file_ids",
    "expires_after",
    "chunking_strategy",
    "metadata",
];

// VectorStoreSearchOptionalRequestParams: the only keys a search body may carry
const SEARCH_FIELDS: [&str; 4] = [
    "filters",
    "max_num_results",
    "ranking_options",
    "rewrite_query",
];

// B1058 over an identity map_openai_params: the declared keys whose values are not null
pub(crate) struct Filters;

impl OptionalParamFilters for Filters {
    fn create_params(&self, params: &Params) -> Params {
        declared(params, &CREATE_FIELDS)
    }

    fn search_params(&self, params: &Params, _tag: VectorStoresConfigTag) -> Params {
        declared(params, &SEARCH_FIELDS)
    }
}

fn declared(params: &Params, names: &[&str]) -> Params {
    params
        .iter()
        .filter(|(name, value)| names.contains(&name.as_str()) && !matches!(value, Json::Null))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

// B010's configs for the tags this build serves; every other provider has no vector store config yet
pub(crate) struct Configs {
    credentials: OpenAiCredentialsSettings,
    globals: LiteLlmGlobals,
    env_api_key: Option<SecretString>,
}

impl Configs {
    pub(crate) fn new(
        credentials: OpenAiCredentialsSettings,
        globals: LiteLlmGlobals,
        env_api_key: Option<SecretString>,
    ) -> Self {
        Self {
            credentials,
            globals,
            env_api_key,
        }
    }
}

impl VectorStoreConfigs for Configs {
    fn create_config(&self, tag: VectorStoresConfigTag) -> Option<Box<dyn CreateReadConfig + '_>> {
        match tag {
            VectorStoresConfigTag::OpenAI => Some(Box::new(OpenAiCreateReadConfig::new(
                &self.credentials,
                globals(&self.globals),
                self.env_api_key.clone(),
                None,
            ))),
            VectorStoresConfigTag::AzureOpenAI
            | VectorStoresConfigTag::Vertex
            | VectorStoresConfigTag::VertexSearchAPI
            | VectorStoresConfigTag::Bedrock
            | VectorStoresConfigTag::PGVector
            | VectorStoresConfigTag::AzureAI
            | VectorStoresConfigTag::Milvus
            | VectorStoresConfigTag::Gemini
            | VectorStoresConfigTag::RAGFlow
            | VectorStoresConfigTag::S3Vectors
            | VectorStoresConfigTag::Valkey
            | VectorStoresConfigTag::MongoDB => None,
        }
    }

    fn search_config(&self, tag: VectorStoresConfigTag) -> Option<Arc<dyn SearchConfig>> {
        match tag {
            VectorStoresConfigTag::OpenAI => Some(Arc::new(OpenAiSearchConfig::new(
                self.credentials.clone(),
                globals(&self.globals),
                self.env_api_key.clone(),
                None,
            ))),
            VectorStoresConfigTag::AzureOpenAI
            | VectorStoresConfigTag::Vertex
            | VectorStoresConfigTag::VertexSearchAPI
            | VectorStoresConfigTag::Bedrock
            | VectorStoresConfigTag::PGVector
            | VectorStoresConfigTag::AzureAI
            | VectorStoresConfigTag::Milvus
            | VectorStoresConfigTag::Gemini
            | VectorStoresConfigTag::RAGFlow
            | VectorStoresConfigTag::S3Vectors
            | VectorStoresConfigTag::Valkey
            | VectorStoresConfigTag::MongoDB => None,
        }
    }
}

// B527 owns the credential source rules, so the globals are copied into the shape its resolver reads
fn globals(source: &LiteLlmGlobals) -> LiteLlmGlobals {
    LiteLlmGlobals {
        api_base: source.api_base.clone(),
        api_key: source.api_key.clone(),
        openai_key: source.openai_key.clone(),
        organization: source.organization.clone(),
    }
}

// A search without a supplied executor: OpenAI embeds the query on the server, so nothing calls this one
pub(crate) struct Executors;

impl EmbeddingExecutorSource for Executors {
    fn select<'selection>(
        &'selection self,
        supplied: Option<&'selection dyn QueryEmbeddingExecutor>,
        _request_kwargs: &Params,
    ) -> SelectedExecutor<'selection> {
        match supplied {
            Some(executor) => SelectedExecutor::Supplied(executor),
            None => SelectedExecutor::Built(Box::new(UnavailableExecutor)),
        }
    }
}

struct UnavailableExecutor;

// The executor B1059 will replace: a search that embeds on the client fails until its embedding lands
#[derive(Debug, thiserror::Error)]
#[error("query embedding needs B1059, which this build does not have yet")]
struct Unavailable;

#[async_trait]
impl QueryEmbeddingExecutor for UnavailableExecutor {
    async fn embed(
        &self,
        _entry: EmbeddingEntry,
        _model: &str,
        _query: &str,
        _configuration: &Params,
    ) -> Result<EmbeddingResponse, StoreFailure> {
        Err(StoreFailure::from(Unavailable))
    }
}

// B595's seam with no signer behind it: every served config's sign_request returns its headers unchanged
pub(crate) struct Inline;

impl SigningSeam for Inline {
    fn sign(&self, config: Arc<dyn SearchConfig>, request: SignInput) -> SignFuture<'_> {
        let signed = config.sign_request(request.request());
        Box::pin(ready(Ok(signed)))
    }
}
