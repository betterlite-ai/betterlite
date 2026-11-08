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
use betterlite_openai::embedding::{EmbeddingError, EmbeddingResponse};
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
use rstest::rstest;
use uuid::Uuid;

use crate::api::vector_store_contracts::executor::DirectEmbeddingExecutor;
use crate::api::vector_store_contracts::router::VectorStoreRouter;
use crate::mock::{chat::response as mock_chat, embedding::response as mock_embedding};

use super::query::{SelectedExecutor, embed_query, query_embedding_executor};

const TWO_RECORDS: &str = r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.25,0.5]},{"object":"embedding","index":1,"embedding":[9,9]}],"model":"text-embedding-3-small","usage":{"prompt_tokens":1,"total_tokens":1}}"#;
const NO_RECORDS: &str = r#"{"object":"list","data":[],"model":"text-embedding-3-small","usage":{"prompt_tokens":1,"total_tokens":1}}"#;

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

// an executor that decodes its body as the real one does, or fails when it holds none
struct ScriptedExecutor(Option<String>);

#[async_trait]
impl VectorStoreEmbeddingExecutor for ScriptedExecutor {
    async fn embed(
        &self,
        _entry: EmbeddingEntry,
        _model: &str,
        _query: &str,
        _configuration: &Params,
    ) -> Result<EmbeddingResponse, VectorStoreEmbeddingError> {
        let Some(body) = &self.0 else {
            return Err(VectorStoreEmbeddingError::DuplicateArgument {
                name: "input".to_owned(),
            });
        };
        decode(body)
    }
}

fn decode(body: &str) -> Result<EmbeddingResponse, VectorStoreEmbeddingError> {
    let decoded = serde_json::from_str::<EmbeddingResponse>(body);
    decoded.map_err(|source| VectorStoreEmbeddingError::Embedding {
        source: EmbeddingError::Decode { source },
    })
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
        model: &str,
        _input: &[String],
        _configuration: Params,
    ) -> Result<EmbeddingResponse, BaseLlmException> {
        Err(BaseLlmException {
            status_code: 404,
            message: format!("no deployment for {model}"),
            headers: BTreeMap::new(),
        })
    }
}

#[test]
fn executor_precedence_prefers_explicit_then_router_then_direct() {
    let explicit = ScriptedExecutor(Some(TWO_RECORDS.to_owned()));
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
    let executor = ScriptedExecutor(Some(TWO_RECORDS.to_owned()));
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
        Some(&ScriptedExecutor(None)),
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
        | QueryEmbeddingError::Answer { .. }
        | QueryEmbeddingError::NotAVector => panic!("unexpected error category"),
    }
    let missing = embed_query(
        EmbeddingEntry::Embedding,
        "a b",
        &Params::new(),
        Some(&ScriptedExecutor(None)),
        direct_executor(),
    )
    .await
    .expect_err("missing model");
    assert!(matches!(missing, QueryEmbeddingError::MissingModel));
}

#[tokio::test]
async fn embed_query_validates_the_answer_after_the_call() {
    let empty = ScriptedExecutor(Some(NO_RECORDS.to_owned()));
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

// the pinned vector validation runs after the call, so a body shape the typed record refuses is no execution failure
#[rstest]
#[case::nonnumeric_text(r#"[{"object":"embedding","index":0,"embedding":["bad"]}]"#)]
#[case::numeric_text(r#"[{"object":"embedding","index":0,"embedding":["0.5","1"]}]"#)]
#[case::no_embedding("[{}]")]
#[case::no_index(r#"[{"object":"embedding","embedding":[0.5]}]"#)]
#[tokio::test]
async fn embed_query_does_not_wrap_a_refused_body_shape(#[case] data: &str) {
    let body = format!(
        r#"{{"object":"list","data":{data},"model":"m","usage":{{"prompt_tokens":1,"total_tokens":1}}}}"#
    );
    let litellm_params = params(r#"{"litellm_embedding_model": "openai/text-embedding-3-small"}"#);
    let error = embed_query(
        EmbeddingEntry::Embedding,
        "a b",
        &litellm_params,
        Some(&ScriptedExecutor(Some(body))),
        direct_executor(),
    )
    .await
    .expect_err("refused body");
    assert!(matches!(error, QueryEmbeddingError::Answer { .. }));
    assert!(!error.to_string().starts_with("Failed to generate"));
}

#[tokio::test]
async fn embed_query_wraps_a_body_that_is_not_json() {
    let litellm_params = params(r#"{"litellm_embedding_model": "openai/text-embedding-3-small"}"#);
    let error = embed_query(
        EmbeddingEntry::Embedding,
        "a b",
        &litellm_params,
        Some(&ScriptedExecutor(Some("<html>".to_owned()))),
        direct_executor(),
    )
    .await
    .expect_err("not json");
    assert!(matches!(error, QueryEmbeddingError::Execution { .. }));
}
