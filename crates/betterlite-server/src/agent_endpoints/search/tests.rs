#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_core::agents::AgentResponse;
use betterlite_semantic::{EmbedError, Embedder, Vector};
use rust_decimal::Decimal;
use serde_json::{from_str, to_string};

use super::index::{AgentSearchEmbedding, AgentSearchIndex};
use super::rank::{
    AgentSearchErrorBody, AgentSearchHttpError, AgentSearchRouteError, rank_agents_by_query,
};
use super::{
    AgentEmbedders, AgentSearchError, AgentSearchHit, AgentSearchOutcome, AgentSearchResult,
    AgentSearchRuntime, DEFAULT_AGENT_SEARCH_TOP_K, SearchableSkill, agent_search_result,
    agent_search_text, search_agents,
};
use crate::auth::auth_identity::AuthContext;

const MODEL_REASON: &str = "agent search needs litellm_settings.agent_search_embedding_model set to an embedding model from model_list";
const ROUTER_REASON: &str = "agent search needs a model_list so the embedding model can be called";

fn caller() -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::ServiceAccount,
            subject: "oracle".to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles: Vec::new(),
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey(None),
        },
    }
}

fn named_agent(agent_id: &str, agent_name: &str, card: &str) -> AgentResponse {
    let source = format!(
        r#"{{"agent_id":"{agent_id}","agent_name":"{agent_name}","agent_card_params":{card}}}"#
    );
    from_str(&source).unwrap()
}

fn agent(agent_id: &str) -> AgentResponse {
    named_agent(agent_id, agent_id, "{}")
}

#[derive(Clone)]
enum EmbeddingFailure {
    Message(String),
    Http { status_code: u16, detail: String },
}

#[derive(Clone)]
struct FixedEmbedder {
    vectors: BTreeMap<String, Vector>,
    fallback: Option<Vector>,
    failure: Option<EmbeddingFailure>,
    calls: Arc<Mutex<Vec<Vec<String>>>>,
}

impl FixedEmbedder {
    fn mapped(entries: &[(&str, Vector)]) -> Self {
        Self {
            vectors: entries
                .iter()
                .map(|(text, vector)| ((*text).to_owned(), vector.clone()))
                .collect(),
            fallback: None,
            failure: None,
            calls: Arc::default(),
        }
    }

    fn uniform(vector: Vector) -> Self {
        Self {
            vectors: BTreeMap::new(),
            fallback: Some(vector),
            failure: None,
            calls: Arc::default(),
        }
    }

    fn failing(failure: EmbeddingFailure) -> Self {
        Self {
            vectors: BTreeMap::new(),
            fallback: None,
            failure: Some(failure),
            calls: Arc::default(),
        }
    }

    fn call_texts(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl Embedder for FixedEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vector>, EmbedError> {
        self.calls.lock().unwrap().push(texts.to_vec());
        if let Some(failure) = &self.failure {
            return Err(match failure {
                EmbeddingFailure::Message(message) => EmbedError::Other(message.clone()),
                EmbeddingFailure::Http {
                    status_code,
                    detail,
                } => EmbedError::Http {
                    status_code: *status_code,
                    detail: detail.clone(),
                },
            });
        }
        texts
            .iter()
            .map(|text| {
                self.vectors
                    .get(text)
                    .cloned()
                    .or_else(|| self.fallback.clone())
                    .ok_or_else(|| EmbedError::Other(format!("no vector for {text}")))
            })
            .collect()
    }
}

struct TestEmbedders {
    embedder: Option<FixedEmbedder>,
    requested: Mutex<Vec<String>>,
}

impl TestEmbedders {
    fn present(embedder: FixedEmbedder) -> Self {
        Self {
            embedder: Some(embedder),
            requested: Mutex::new(Vec::new()),
        }
    }

    fn missing() -> Self {
        Self {
            embedder: None,
            requested: Mutex::new(Vec::new()),
        }
    }

    fn requested_models(&self) -> Vec<String> {
        self.requested.lock().unwrap().clone()
    }
}

impl AgentEmbedders for TestEmbedders {
    fn embedder_for(
        &self,
        embedding_model: &str,
        _caller: &AuthContext,
    ) -> Option<Box<dyn Embedder>> {
        self.requested
            .lock()
            .unwrap()
            .push(embedding_model.to_owned());
        let stored = self.embedder.as_ref()?;
        Some(Box::new(stored.clone()))
    }
}

async fn search_once(
    embedders: &TestEmbedders,
    index: &AgentSearchIndex,
    caller: &AuthContext,
    embedding_model: &str,
) {
    let agents = vec![agent("a1"), agent("a2")];
    search_agents(
        "query",
        &agents,
        5,
        Some(embedding_model),
        AgentSearchRuntime {
            embedders,
            index,
            caller,
        },
    )
    .await
    .unwrap();
}

#[test]
fn text_joins_name_description_and_skill_lines() {
    let source = named_agent(
        "a1",
        "planner",
        r#"{"description":"Plans trips","skills":[{"name":"lookup","description":"Finds places","tags":["travel","maps"]},{"name":"summarize"},{"description":"no name"},{}]}"#,
    );
    assert_eq!(
        agent_search_text(&source),
        "planner\nPlans trips\nlookup Finds places travel maps\nsummarize\nno name"
    );
}

#[test]
fn text_skips_every_empty_part() {
    let empty = named_agent("a1", "", r#"{"description":"","skills":[{}]}"#);
    assert_eq!(agent_search_text(&empty), "");
    let worker = named_agent("a1", "worker", r#"{"description":""}"#);
    assert_eq!(agent_search_text(&worker), "worker");
}

#[test]
fn result_projects_only_the_agent_identity_card_and_score() {
    let source = named_agent(
        "a1",
        "planner",
        r#"{"description":"Plans trips","skills":[{"id":"s1","name":"lookup","description":"Finds places","tags":["travel"],"examples":["find Paris"],"inputModes":["text"],"outputModes":["text"]}]}"#,
    );
    let hit = AgentSearchHit {
        agent: source,
        score: 0.75,
    };
    let result = agent_search_result(&hit);
    assert_eq!(result.agent_id, "a1");
    assert_eq!(result.agent_name, "planner");
    assert_eq!(result.description, "Plans trips");
    assert_eq!(
        result.skills,
        vec![SearchableSkill {
            name: "lookup".to_owned(),
            description: "Finds places".to_owned(),
            tags: vec!["travel".to_owned()],
        }]
    );
    assert_eq!(result.score.to_bits(), 0.75_f64.to_bits());
    assert_eq!(
        to_string(&result).unwrap(),
        r#"{"agent_id":"a1","agent_name":"planner","description":"Plans trips","skills":[{"name":"lookup","description":"Finds places","tags":["travel"]}],"score":0.75}"#
    );
}

#[test]
fn result_never_carries_agent_parameters_or_headers() {
    let source = from_str::<AgentResponse>(
        r#"{"agent_id":"a1","agent_name":"planner","agent_card_params":{},"litellm_params":{"api_key":"sk-secret"},"static_headers":{"authorization":"Bearer secret"},"kill_switch":{"url":"https://agent.example/stop"}}"#,
    )
    .unwrap();
    let result = agent_search_result(&AgentSearchHit {
        agent: source,
        score: 1.0,
    });
    let serialized = to_string(&result).unwrap();
    assert!(!serialized.contains("sk-secret"));
    assert!(!serialized.contains("Bearer secret"));
    assert_eq!(
        serialized,
        r#"{"agent_id":"a1","agent_name":"planner","description":"","skills":[],"score":1.0}"#
    );
}

#[tokio::test]
async fn index_ranks_by_descending_score_and_keeps_ties_in_input_order() {
    let embedder = FixedEmbedder::mapped(&[
        ("query", vec![1.0_f64, 0.0_f64]),
        ("a1", vec![0.0_f64, 1.0_f64]),
        ("a2", vec![1.0_f64, 0.0_f64]),
        ("a3", vec![1.0_f64, 0.0_f64]),
    ]);
    let index = AgentSearchIndex::default();
    let agents = vec![agent("a1"), agent("a2"), agent("a3")];
    let outcome = index
        .search(
            "query",
            &agents,
            2,
            AgentSearchEmbedding {
                embedder: &embedder,
                embedding_model: "model",
            },
        )
        .await
        .unwrap();
    let AgentSearchOutcome::Hits(hits) = outcome else {
        panic!("expected hits");
    };
    assert_eq!(
        hits.iter()
            .map(|hit| hit.agent.agent_id.as_str())
            .collect::<Vec<_>>(),
        ["a2", "a3"]
    );
    assert_eq!(hits[0].score.to_bits(), 1.0_f64.to_bits());
    assert_eq!(hits[1].score.to_bits(), 1.0_f64.to_bits());
}

#[tokio::test]
async fn index_returns_empty_hits_without_agents_or_embeddings() {
    let embedder = FixedEmbedder::uniform(vec![1.0_f64, 0.0_f64]);
    let index = AgentSearchIndex::default();
    let outcome = index
        .search(
            "query",
            &[],
            DEFAULT_AGENT_SEARCH_TOP_K,
            AgentSearchEmbedding {
                embedder: &embedder,
                embedding_model: "model",
            },
        )
        .await
        .unwrap();
    assert!(matches!(outcome, AgentSearchOutcome::Hits(hits) if hits.is_empty()));
    assert!(embedder.call_texts().is_empty());
}

#[tokio::test]
async fn index_maps_a_failed_embedding_to_the_failed_outcome() {
    let embedder = FixedEmbedder::failing(EmbeddingFailure::Message("boom".to_owned()));
    let index = AgentSearchIndex::default();
    let outcome = index
        .search(
            "query",
            &[agent("a1")],
            5,
            AgentSearchEmbedding {
                embedder: &embedder,
                embedding_model: "model",
            },
        )
        .await
        .unwrap();
    match outcome {
        AgentSearchOutcome::EmbeddingFailed { reason } => {
            assert_eq!(reason, "embedding the search query failed: boom");
        }
        AgentSearchOutcome::Hits(_) | AgentSearchOutcome::NotConfigured { .. } => {
            panic!("expected an embedding failure");
        }
    }
}

#[tokio::test]
async fn index_passes_an_embedding_http_error_up() {
    let embedder = FixedEmbedder::failing(EmbeddingFailure::Http {
        status_code: 429,
        detail: "slow down".to_owned(),
    });
    let index = AgentSearchIndex::default();
    let error = index
        .search(
            "query",
            &[agent("a1")],
            5,
            AgentSearchEmbedding {
                embedder: &embedder,
                embedding_model: "model",
            },
        )
        .await
        .unwrap_err();
    match error {
        AgentSearchError::Http {
            status_code,
            detail,
        } => {
            assert_eq!(status_code, 429);
            assert_eq!(detail, "slow down");
        }
    }
}

#[tokio::test]
async fn search_agents_checks_the_embedding_model_before_the_embedder() {
    let embedders = TestEmbedders::present(FixedEmbedder::uniform(vec![1.0_f64, 0.0_f64]));
    let index = AgentSearchIndex::default();
    let caller = caller();
    let runtime = AgentSearchRuntime {
        embedders: &embedders,
        index: &index,
        caller: &caller,
    };
    let outcome = search_agents("query", &[agent("a1")], 5, None, runtime)
        .await
        .unwrap();
    match outcome {
        AgentSearchOutcome::NotConfigured { reason } => assert_eq!(reason, MODEL_REASON),
        AgentSearchOutcome::Hits(_) | AgentSearchOutcome::EmbeddingFailed { .. } => {
            panic!("expected not configured");
        }
    }
    assert!(embedders.requested_models().is_empty());
}

#[tokio::test]
async fn search_agents_reports_a_missing_model_list() {
    let embedders = TestEmbedders::missing();
    let index = AgentSearchIndex::default();
    let caller = caller();
    let runtime = AgentSearchRuntime {
        embedders: &embedders,
        index: &index,
        caller: &caller,
    };
    let outcome = search_agents("query", &[agent("a1")], 5, Some("model"), runtime)
        .await
        .unwrap();
    match outcome {
        AgentSearchOutcome::NotConfigured { reason } => assert_eq!(reason, ROUTER_REASON),
        AgentSearchOutcome::Hits(_) | AgentSearchOutcome::EmbeddingFailed { .. } => {
            panic!("expected not configured");
        }
    }
    assert_eq!(embedders.requested_models(), ["model"]);
}

#[tokio::test]
async fn search_agents_returns_hits() {
    let embedder = FixedEmbedder::mapped(&[
        ("query", vec![1.0_f64, 0.0_f64]),
        ("a1", vec![1.0_f64, 0.0_f64]),
    ]);
    let embedders = TestEmbedders::present(embedder);
    let index = AgentSearchIndex::default();
    let caller = caller();
    let runtime = AgentSearchRuntime {
        embedders: &embedders,
        index: &index,
        caller: &caller,
    };
    let outcome = search_agents("query", &[agent("a1")], 5, Some("model"), runtime)
        .await
        .unwrap();
    let AgentSearchOutcome::Hits(hits) = outcome else {
        panic!("expected hits");
    };
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].agent.agent_id, "a1");
    assert_eq!(hits[0].score.to_bits(), 1.0_f64.to_bits());
}

#[tokio::test]
async fn repeated_search_embeds_only_the_query() {
    let embedder = FixedEmbedder::uniform(vec![1.0_f64, 0.0_f64]);
    let embedders = TestEmbedders::present(embedder.clone());
    let index = AgentSearchIndex::default();
    let caller = caller();
    search_once(&embedders, &index, &caller, "model-a").await;
    search_once(&embedders, &index, &caller, "model-a").await;
    search_once(&embedders, &index, &caller, "model-b").await;
    search_once(&embedders, &index, &caller, "model-a").await;
    assert_eq!(
        embedder.call_texts(),
        [
            vec!["query".to_owned(), "a1".to_owned(), "a2".to_owned()],
            vec!["query".to_owned()],
            vec!["query".to_owned(), "a1".to_owned(), "a2".to_owned()],
            vec!["query".to_owned()],
        ]
    );
}

#[tokio::test]
async fn rank_agents_by_query_sets_search_score_in_rank_order() {
    let embedder = FixedEmbedder::mapped(&[
        ("query", vec![1.0_f64, 0.0_f64]),
        ("a1", vec![0.0_f64, 1.0_f64]),
        ("a2", vec![1.0_f64, 0.0_f64]),
    ]);
    let embedders = TestEmbedders::present(embedder);
    let index = AgentSearchIndex::default();
    let caller = caller();
    let agents = vec![agent("a1"), agent("a2")];
    let runtime = AgentSearchRuntime {
        embedders: &embedders,
        index: &index,
        caller: &caller,
    };
    let ranked = rank_agents_by_query("query", &agents, 5, Some("model"), runtime)
        .await
        .unwrap();
    assert_eq!(
        ranked
            .iter()
            .map(|agent| agent.agent_id.as_str())
            .collect::<Vec<_>>(),
        ["a2", "a1"]
    );
    assert_eq!(
        ranked[0].search_score,
        Some(from_str::<Decimal>("1").unwrap())
    );
    assert_eq!(
        ranked[1].search_score,
        Some(from_str::<Decimal>("0").unwrap())
    );
    assert!(agents.iter().all(|agent| agent.search_score.is_none()));
}

#[tokio::test]
async fn rank_agents_by_query_keeps_the_shortest_score_text() {
    let embedder = FixedEmbedder::mapped(&[
        ("query", vec![1.0_f64, 0.0_f64]),
        ("a1", vec![0.6_f64, 0.8_f64]),
    ]);
    let embedders = TestEmbedders::present(embedder);
    let index = AgentSearchIndex::default();
    let caller = caller();
    let runtime = AgentSearchRuntime {
        embedders: &embedders,
        index: &index,
        caller: &caller,
    };
    let ranked = rank_agents_by_query("query", &[agent("a1")], 5, Some("model"), runtime)
        .await
        .unwrap();
    assert_eq!(
        ranked[0].search_score,
        Some(from_str::<Decimal>("0.6").unwrap())
    );
}

#[tokio::test]
async fn rank_agents_by_query_maps_a_non_finite_score_to_a_500() {
    let embedder =
        FixedEmbedder::mapped(&[("query", vec![1.0e200_f64]), ("a1", vec![1.0e200_f64])]);
    let embedders = TestEmbedders::present(embedder);
    let index = AgentSearchIndex::default();
    let caller = caller();
    let runtime = AgentSearchRuntime {
        embedders: &embedders,
        index: &index,
        caller: &caller,
    };
    let error = rank_agents_by_query("query", &[agent("a1")], 5, Some("model"), runtime)
        .await
        .unwrap_err();
    match error {
        AgentSearchRouteError::Embedding { status_code, .. } => assert_eq!(status_code, 500),
        AgentSearchRouteError::Http(_) => panic!("expected the embedding error"),
    }
}

#[tokio::test]
async fn rank_agents_by_query_maps_not_configured_to_a_400() {
    let embedders = TestEmbedders::missing();
    let index = AgentSearchIndex::default();
    let caller = caller();
    let runtime = AgentSearchRuntime {
        embedders: &embedders,
        index: &index,
        caller: &caller,
    };
    let error = rank_agents_by_query("query", &[agent("a1")], 5, None, runtime)
        .await
        .unwrap_err();
    match error {
        AgentSearchRouteError::Http(http) => {
            assert_eq!(http.status_code, 400);
            assert_eq!(http.error, "agent_search_not_configured");
            assert_eq!(http.message, MODEL_REASON);
        }
        AgentSearchRouteError::Embedding { .. } => panic!("expected an HTTP error"),
    }
}

#[tokio::test]
async fn rank_agents_by_query_maps_a_failed_embedding_to_a_503() {
    let embedders = TestEmbedders::present(FixedEmbedder::failing(EmbeddingFailure::Message(
        "boom".to_owned(),
    )));
    let index = AgentSearchIndex::default();
    let caller = caller();
    let runtime = AgentSearchRuntime {
        embedders: &embedders,
        index: &index,
        caller: &caller,
    };
    let error = rank_agents_by_query("query", &[agent("a1")], 5, Some("model"), runtime)
        .await
        .unwrap_err();
    match error {
        AgentSearchRouteError::Http(http) => {
            assert_eq!(http.status_code, 503);
            assert_eq!(http.error, "agent_search_unavailable");
            assert_eq!(http.message, "embedding the search query failed: boom");
        }
        AgentSearchRouteError::Embedding { .. } => panic!("expected an HTTP error"),
    }
}

#[tokio::test]
async fn rank_agents_by_query_passes_an_embedding_http_error_up() {
    let embedders = TestEmbedders::present(FixedEmbedder::failing(EmbeddingFailure::Http {
        status_code: 401,
        detail: "bad key".to_owned(),
    }));
    let index = AgentSearchIndex::default();
    let caller = caller();
    let runtime = AgentSearchRuntime {
        embedders: &embedders,
        index: &index,
        caller: &caller,
    };
    let error = rank_agents_by_query("query", &[agent("a1")], 5, Some("model"), runtime)
        .await
        .unwrap_err();
    match error {
        AgentSearchRouteError::Embedding {
            status_code,
            detail,
        } => {
            assert_eq!(status_code, 401);
            assert_eq!(detail, "bad key");
        }
        AgentSearchRouteError::Http(_) => panic!("expected the embedding error"),
    }
}

#[test]
fn agent_search_http_error_renders_detail_error_and_message() {
    let body = AgentSearchErrorBody::from(&AgentSearchHttpError {
        status_code: 400,
        error: "agent_search_not_configured".to_owned(),
        message: "the reason".to_owned(),
    });
    assert_eq!(
        to_string(&body).unwrap(),
        r#"{"detail":{"error":"agent_search_not_configured","message":"the reason"}}"#
    );
}

#[test]
fn the_result_record_exposes_the_projection_fields() {
    let result = AgentSearchResult {
        agent_id: "a1".to_owned(),
        agent_name: "planner".to_owned(),
        description: "Plans".to_owned(),
        skills: Vec::new(),
        score: 0.5,
    };
    assert_eq!(result.agent_id, "a1");
    assert_eq!(result.score.to_bits(), 0.5_f64.to_bits());
}
