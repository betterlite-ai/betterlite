use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::{ModelDeployment, ProxyTrust};
use betterlite_core::api::vector_store_contracts::executor::DirectEmbeddingExecutor;
use betterlite_core::api::vector_store_contracts::router::VectorStoreRouter;
use betterlite_core::mock::{chat::response as mock_chat, embedding::response as mock_embedding};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreCreateResponse, VectorStoreSearchResponse,
};
use betterlite_openai::chat::MockSettings;
use betterlite_openai::chat::deployment::OpenAiDefaults;
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_provider::api::vector_query_embedding::direct::{
    DirectSearchRequest, DirectVectorStoreConfig,
};
use betterlite_provider::api::vector_query_embedding::error::DirectStoreError;
use betterlite_provider::api::vector_query_embedding::query::{
    QueryEmbeddingConfig, QuerySearchRequest,
};
use betterlite_provider::api::vector_store_contracts::config::{
    AuthCredentials, EmbeddingEntry, EndpointsByType, Params, PreparedRequest, SearchQuery,
    SearchRequest, VectorStoreConfig, VectorStoreEmbeddingExecutor, VectorStoreLogging,
    VectorStoreResponse,
};
use betterlite_provider::api::vector_store_contracts::error::{
    BaseLlmException, VectorStoreContractError, VectorStoreEmbeddingError,
};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use serde_json::Number;
use thiserror::Error;
use uuid::Uuid;

use crate::helpers::HelperError;

#[derive(Debug, Error)]
pub(super) enum FixtureError {
    #[error(transparent)]
    Contract(#[from] VectorStoreContractError),
}

const fn contract_error() -> FixtureError {
    FixtureError::Contract(VectorStoreContractError::ApiBaseRequired)
}

pub(super) fn helper_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn direct_executor() -> Result<DirectEmbeddingExecutor, Box<HelperError>> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .map_err(|error| helper_error(error.to_string()))?,
    );
    let client =
        ProxiedClient::outbound(&policy).map_err(|error| helper_error(error.to_string()))?;
    Ok(DirectEmbeddingExecutor {
        executor: ChatExecutor {
            client,
            deployments: Vec::new(),
            catalog: CatalogCapabilities {
                chat_models: BTreeSet::new(),
                text_models: BTreeSet::new(),
            },
            random: Mutex::new(StdRng::seed_from_u64(0)),
            clock: || 0,
            next_id: Uuid::new_v4,
            mock: MockSettings::default(),
            mock_chat,
            mock_embedding,
        },
        deployments: Vec::new(),
        defaults: OpenAiDefaults::default(),
        catalog: BundledCatalog::default().get(),
    })
}

pub(super) struct ExplicitExecutor;

#[async_trait]
impl VectorStoreEmbeddingExecutor for ExplicitExecutor {
    async fn embed(
        &self,
        _entry: EmbeddingEntry,
        _model: &str,
        _query: &str,
        _configuration: &Params,
    ) -> Result<EmbeddingResponse, VectorStoreEmbeddingError> {
        Err(VectorStoreEmbeddingError::DuplicateArgument {
            name: "the precedence fixture never runs".to_owned(),
        })
    }
}

pub(super) struct RouterFixture;

#[async_trait]
impl VectorStoreRouter for RouterFixture {
    fn resolved_litellm_models(&self, _model: &str, _team_id: Option<&str>) -> Vec<String> {
        Vec::new()
    }

    fn model_list(&self) -> Vec<ModelDeployment> {
        Vec::new()
    }

    async fn embedding(
        &self,
        _entry: EmbeddingEntry,
        _model: &str,
        _input: &[String],
        _configuration: Params,
    ) -> Result<EmbeddingResponse, BaseLlmException> {
        Err(BaseLlmException {
            status_code: 404,
            message: "no deployment".to_owned(),
            headers: Params::new(),
        })
    }
}

pub(super) struct DirectFixture;

impl DirectVectorStoreConfig for DirectFixture {
    fn execute_search_request(
        &self,
        _request: DirectSearchRequest<'_>,
    ) -> Result<VectorStoreSearchResponse, DirectStoreError> {
        Ok(VectorStoreSearchResponse::default())
    }
}

pub(super) struct QueryFixture;

impl VectorStoreConfig for QueryFixture {
    type Error = FixtureError;

    fn auth_credentials(&self, _litellm_params: &Params) -> AuthCredentials {
        AuthCredentials::default()
    }

    fn endpoints_by_type(&self) -> EndpointsByType {
        EndpointsByType::default()
    }

    fn transform_search_request(
        &self,
        _request: SearchRequest<'_>,
    ) -> Result<PreparedRequest, FixtureError> {
        Err(contract_error())
    }

    fn transform_search_response(
        &self,
        _response: &VectorStoreResponse,
        _logging: &mut VectorStoreLogging,
    ) -> Result<VectorStoreSearchResponse, FixtureError> {
        Err(contract_error())
    }

    fn transform_create_request(
        &self,
        _optional_params: &Params,
        _api_base: &str,
    ) -> Result<PreparedRequest, FixtureError> {
        Err(contract_error())
    }

    fn transform_create_response(
        &self,
        _response: &VectorStoreResponse,
    ) -> Result<VectorStoreCreateResponse, FixtureError> {
        Err(contract_error())
    }
}

// every argument LiteLLM passes, in the order it passes them
fn arguments(search: &SearchRequest<'_>) -> Json {
    let query = match search.query {
        SearchQuery::Text(text) => Json::Text(text.clone()),
        SearchQuery::Sequence(parts) => {
            Json::List(parts.iter().map(|part| Json::Text(part.clone())).collect())
        }
    };
    let literal = |params: &Params| {
        Json::Object(
            params
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
        )
    };
    let argument = |name: &str, value: Json| Json::List(vec![Json::Text(name.to_owned()), value]);
    Json::List(vec![
        argument("vector_store_id", Json::Text(search.vector_store_id.to_owned())),
        argument("query", query),
        argument("optional_params", literal(search.optional_params)),
        argument("api_base", Json::Text(search.api_base.to_owned())),
        argument("litellm_params", literal(search.litellm_params)),
        argument("extra_body", search.extra_body.map_or(Json::Null, literal)),
    ])
}

impl QueryFixture {
    // the fixture reports what it received and counts the call in the logging object
    pub(super) fn forward(request: QuerySearchRequest<'_>) -> PreparedRequest {
        let QuerySearchRequest {
            search,
            embedding_executor,
        } = request;
        let body = Params::from([
            ("arguments".to_owned(), arguments(&search)),
            (
                "embedding_executor".to_owned(),
                Json::Text(match embedding_executor {
                    Some(_) => "explicit",
                    None => "none",
                }
                .to_owned()),
            ),
        ]);
        let calls = match search.logging.model_call_details.get("calls") {
            Some(Json::Number(number)) => number.as_u64().unwrap_or(0),
            _ => 0,
        }
        .saturating_add(1);
        search
            .logging
            .model_call_details
            .insert("calls".to_owned(), Json::Number(Number::from(calls)));
        (format!("{}/vector_stores/search", search.api_base), body)
    }
}

impl QueryEmbeddingConfig for QueryFixture {
    fn transform_query_search_request(
        &self,
        request: QuerySearchRequest<'_>,
    ) -> Result<PreparedRequest, FixtureError> {
        Ok(Self::forward(request))
    }
}
