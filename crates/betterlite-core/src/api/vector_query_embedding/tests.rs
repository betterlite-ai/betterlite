#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::{ModelDeployment, ProxyTrust};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
use betterlite_openai::chat::MockSettings;
use betterlite_openai::chat::deployment::OpenAiDefaults;
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_provider::api::vector_query_embedding::error::QueryEmbeddingError;
use betterlite_provider::api::vector_store_contracts::config::{
    EmbeddingEntry, Params, VectorStoreEmbeddingExecutor,
};
use betterlite_provider::api::vector_store_contracts::error::{
    BaseLlmException, VectorStoreEmbeddingError,
};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use uuid::Uuid;

use crate::api::vector_store_contracts::executor::DirectEmbeddingExecutor;
use crate::api::vector_store_contracts::router::VectorStoreRouter;
use crate::mock::{chat::response as mock_chat, embedding::response as mock_embedding};

use super::query::{SelectedExecutor, embed_query, query_embedding_executor};

const TWO_RECORDS: &str = r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.25,0.5]},{"object":"embedding","index":1,"embedding":[9,9]}],"model":"text-embedding-3-small","usage":{"prompt_tokens":1,"total_tokens":1}}"#;
const NO_RECORDS: &str = r#"{"object":"list","data":[],"model":"text-embedding-3-small","usage":{"prompt_tokens":1,"total_tokens":1}}"#;

fn answer(text: &str) -> EmbeddingResponse {
    serde_json::from_str(text).expect("embedding response")
}

fn params(text: &str) -> Params {
    serde_json::from_str(text).expect("params")
}

fn direct_executor() -> DirectEmbeddingExecutor {
    let policy =
        Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("policy"));
    DirectEmbeddingExecutor {
        executor: ChatExecutor {
            client: ProxiedClient::outbound(&policy).expect("client"),
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
    }
}

struct AnsweringExecutor(EmbeddingResponse);

#[async_trait]
impl VectorStoreEmbeddingExecutor for AnsweringExecutor {
    async fn embed(
        &self,
        _entry: EmbeddingEntry,
        _model: &str,
        _query: &str,
        _configuration: &Params,
    ) -> Result<EmbeddingResponse, VectorStoreEmbeddingError> {
        Ok(self.0.clone())
    }
}

struct FailingExecutor;

#[async_trait]
impl VectorStoreEmbeddingExecutor for FailingExecutor {
    async fn embed(
        &self,
        _entry: EmbeddingEntry,
        _model: &str,
        _query: &str,
        _configuration: &Params,
    ) -> Result<EmbeddingResponse, VectorStoreEmbeddingError> {
        Err(VectorStoreEmbeddingError::DuplicateArgument {
            name: "input".to_owned(),
        })
    }
}

struct FakeRouter;

#[async_trait]
impl VectorStoreRouter for FakeRouter {
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
            headers: BTreeMap::new(),
        })
    }
}

#[test]
fn executor_precedence_prefers_explicit_then_router_then_direct() {
    let explicit = AnsweringExecutor(answer(TWO_RECORDS));
    let metadata = params(r#"{"team": "a"}"#);
    assert!(matches!(
        query_embedding_executor(
            Some(&explicit),
            Some((FakeRouter, metadata.clone())),
            direct_executor()
        ),
        SelectedExecutor::Explicit(_)
    ));
    match query_embedding_executor(
        None,
        Some((FakeRouter, metadata.clone())),
        direct_executor(),
    ) {
        SelectedExecutor::Router(router) => assert_eq!(router.metadata, metadata),
        SelectedExecutor::Explicit(_) | SelectedExecutor::Direct(_) => {
            panic!("a router alone selects the router executor")
        }
    }
    assert!(matches!(
        query_embedding_executor::<FakeRouter>(None, None, direct_executor()),
        SelectedExecutor::Direct(_)
    ));
}

#[tokio::test]
async fn embed_query_returns_the_first_vector() {
    let executor = AnsweringExecutor(answer(TWO_RECORDS));
    let litellm_params = params(
        r#"{"litellm_embedding_model": "openai/text-embedding-3-small", "litellm_embedding_config": {"dimensions": 2}}"#,
    );
    let vector = embed_query(
        EmbeddingEntry::Embedding,
        "a b",
        &litellm_params,
        Some(&executor),
        direct_executor(),
    )
    .await
    .expect("vector");
    assert_eq!(vector, vec![0.25_f64, 0.5_f64]);
}

#[tokio::test]
async fn embed_query_wraps_only_the_executor_failure() {
    let litellm_params = params(r#"{"litellm_embedding_model": "openai/text-embedding-3-small"}"#);
    let error = embed_query(
        EmbeddingEntry::Aembedding,
        "a b",
        &litellm_params,
        Some(&FailingExecutor),
        direct_executor(),
    )
    .await
    .expect_err("failure");
    assert!(
        error
            .to_string()
            .starts_with("Failed to generate embedding for query: ")
    );
    match error {
        QueryEmbeddingError::Execution { .. } => {}
        QueryEmbeddingError::MissingModel
        | QueryEmbeddingError::ConfigKey
        | QueryEmbeddingError::NoFirstRecord
        | QueryEmbeddingError::NoEmbedding
        | QueryEmbeddingError::NotAVector => panic!("unexpected error category"),
    }
    let missing = embed_query(
        EmbeddingEntry::Embedding,
        "a b",
        &Params::new(),
        Some(&FailingExecutor),
        direct_executor(),
    )
    .await
    .expect_err("missing model");
    assert!(matches!(missing, QueryEmbeddingError::MissingModel));
}

#[tokio::test]
async fn embed_query_validates_the_answer_after_the_call() {
    let empty = AnsweringExecutor(answer(NO_RECORDS));
    let litellm_params = params(r#"{"litellm_embedding_model": "openai/text-embedding-3-small"}"#);
    let error = embed_query(
        EmbeddingEntry::Embedding,
        "a b",
        &litellm_params,
        Some(&empty),
        direct_executor(),
    )
    .await
    .expect_err("no record");
    assert!(matches!(error, QueryEmbeddingError::NoFirstRecord));
}
