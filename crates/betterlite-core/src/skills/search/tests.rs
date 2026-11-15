#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_context::dynamic_logging_cache::Clock;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::managed_resource_rows::skills::SkillTable;
use betterlite_semantic::{EmbedError, Embedder, Vector};
use serde_json::json;

use super::{
    DEFAULT_SKILL_SEARCH_TOP_K, HostedSkillSearchError, HostedSkillSearchRequest,
    MAX_SKILL_SEARCH_TEXT_CHARS, MAX_SKILL_SEARCH_TOP_K, SkillSearchEmbedding, SkillSearchError,
    SkillSearchHit, SkillSearchIndex, SkillSearchOutcome, SkillSearchRuntime, search_hosted_skills,
    search_skills, skill_search_result, skill_search_text,
};
use crate::skills::store::repository::{SkillRepository, SkillWrite};
use crate::skills::store::{HostedSkillStore, SkillIdSource, SkillPage};

const MODEL: &str = "test-embedding";
const MODEL_MISSING: &str = "skill search needs litellm_settings.skill_search_embedding_model set to an embedding model from model_list";
const ROUTER_MISSING: &str = "skill search needs a model_list so the embedding model can be called";

#[derive(Clone)]
enum Failure {
    Value(String),
    Http { status_code: u16, detail: String },
}

struct FixedEmbedder {
    vectors: BTreeMap<String, Vector>,
    failure: Option<Failure>,
    calls: Mutex<Vec<Vec<String>>>,
}

impl FixedEmbedder {
    fn new(vectors: BTreeMap<String, Vector>) -> Self {
        Self {
            vectors,
            failure: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn failing(failure: Failure) -> Self {
        Self {
            vectors: BTreeMap::new(),
            failure: Some(failure),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().expect("calls lock").clone()
    }
}

#[async_trait]
impl Embedder for FixedEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vector>, EmbedError> {
        self.calls.lock().expect("calls lock").push(texts.to_vec());
        if let Some(failure) = &self.failure {
            return Err(match failure {
                Failure::Value(message) => EmbedError::Other(message.clone()),
                Failure::Http {
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
                    .ok_or_else(|| EmbedError::Other(format!("no vector for {text}")))
            })
            .collect()
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Effect {
    Availability,
    FindMany {
        page: SkillPage,
        created_by: Option<Vec<String>>,
    },
}

#[derive(Default)]
struct State {
    page: Vec<SkillTable>,
    effects: Vec<Effect>,
    page_failure: Option<&'static str>,
}

#[derive(Clone, Default)]
struct ScriptedRepository {
    state: Arc<Mutex<State>>,
}

impl ScriptedRepository {
    fn with_page(page: Vec<SkillTable>) -> Self {
        let repository = Self::default();
        repository.state.lock().expect("state lock").page = page;
        repository
    }

    fn failing_page(message: &'static str) -> Self {
        let repository = Self::default();
        repository.state.lock().expect("state lock").page_failure = Some(message);
        repository
    }

    fn effects(&self) -> Vec<Effect> {
        self.state.lock().expect("state lock").effects.clone()
    }
}

impl SkillRepository for ScriptedRepository {
    fn availability(&self) -> Result<(), TableError> {
        self.state
            .lock()
            .expect("state lock")
            .effects
            .push(Effect::Availability);
        Ok(())
    }

    fn create(
        &self,
        _write: SkillWrite,
    ) -> impl Future<Output = Result<SkillTable, TableError>> + Send {
        ready(Err(TableError::Publish {
            message: "the scripted repository has no create".to_owned(),
        }))
    }

    fn find_unique(
        &self,
        _skill_id: &str,
    ) -> impl Future<Output = Result<Option<SkillTable>, TableError>> + Send {
        ready(Err(TableError::Publish {
            message: "the scripted repository has no find_unique".to_owned(),
        }))
    }

    fn find_many(
        &self,
        page: SkillPage,
        created_by: Option<Vec<String>>,
    ) -> impl Future<Output = Result<Vec<SkillTable>, TableError>> + Send {
        let result = {
            let mut state = self.state.lock().expect("state lock");
            state.effects.push(Effect::FindMany { page, created_by });
            match state.page_failure {
                Some(message) => Err(TableError::Publish {
                    message: message.to_owned(),
                }),
                None => Ok(state.page.clone()),
            }
        };
        ready(result)
    }

    fn delete(&self, _skill_id: &str) -> impl Future<Output = Result<(), TableError>> + Send {
        ready(Err(TableError::Publish {
            message: "the scripted repository has no delete".to_owned(),
        }))
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a runtime")
}

fn skill_row(skill_id: &str, title: Option<&str>, description: Option<&str>) -> SkillTable {
    serde_json::from_value(json!({
        "skill_id": skill_id,
        "display_title": title,
        "description": description,
    }))
    .expect("a scripted skill row")
}

fn vectors(items: &[(&str, [f64; 2])]) -> BTreeMap<String, Vector> {
    items
        .iter()
        .map(|(text, vector)| ((*text).to_owned(), vector.to_vec()))
        .collect()
}

fn ranked_skills() -> Vec<SkillTable> {
    vec![
        skill_row("litellm_skill_a", Some("Alpha"), None),
        skill_row("litellm_skill_b", Some("Beta"), None),
        skill_row("litellm_skill_c", Some("Gamma"), None),
        skill_row("litellm_skill_d", Some("Delta"), None),
    ]
}

fn ranked_vectors() -> BTreeMap<String, Vector> {
    vectors(&[
        ("q", [1.0, 0.0]),
        ("Alpha", [1.0, 1.0]),
        ("Beta", [1.0, 1.0]),
        ("Gamma", [1.0, 3.0]),
        ("Delta", [3.0, 1.0]),
    ])
}

fn ranked(embedder: &FixedEmbedder, top_k: usize) -> SkillSearchOutcome {
    rank_in(
        &SkillSearchIndex::new(10),
        embedder,
        &ranked_skills(),
        top_k,
        MODEL,
    )
    .expect("a ranked search")
}

fn rank_in(
    index: &SkillSearchIndex,
    embedder: &FixedEmbedder,
    skills: &[SkillTable],
    top_k: usize,
    model: &str,
) -> Result<SkillSearchOutcome, SkillSearchError> {
    runtime().block_on(index.search(
        "q",
        skills,
        top_k,
        SkillSearchEmbedding {
            embedder,
            embedding_model: model,
        },
    ))
}

fn search_outcome(
    index: &SkillSearchIndex,
    embedder: Option<&FixedEmbedder>,
    model: Option<&str>,
    skills: &[SkillTable],
) -> SkillSearchOutcome {
    let embedder: Option<&dyn Embedder> = match embedder {
        Some(value) => Some(value),
        None => None,
    };
    runtime()
        .block_on(search_skills(
            "q",
            skills,
            10,
            SkillSearchRuntime {
                embedder,
                embedding_model: model,
                index,
            },
        ))
        .expect("a search outcome")
}

fn hosted_outcome(
    store: &HostedSkillStore<ScriptedRepository>,
    principal: Option<&Principal>,
    provider: Option<&str>,
    embedder: &FixedEmbedder,
    index: &SkillSearchIndex,
) -> Result<SkillSearchOutcome, HostedSkillSearchError> {
    let embedder: Option<&dyn Embedder> = Some(embedder);
    runtime().block_on(search_hosted_skills(
        store,
        principal,
        HostedSkillSearchRequest {
            custom_llm_provider: provider,
            query: "q",
            top_k: 10,
            runtime: SkillSearchRuntime {
                embedder,
                embedding_model: Some(MODEL),
                index,
            },
        },
    ))
}

fn store_of(repository: ScriptedRepository) -> HostedSkillStore<ScriptedRepository> {
    let ids: SkillIdSource = Arc::new(|| "litellm_skill_test".to_owned());
    let clock: Clock = Arc::new(|| 1_000.0_f64);
    HostedSkillStore::new(repository, ids, clock).expect("a hosted skill store")
}

fn principal() -> Principal {
    serde_json::from_str(
        r#"{"principal_type":"human","subject":"alice","auth_method":"api_key","user":{"id":"alice"},"teams":[{"id":"team-1"}],"organization":{"id":"org-1"},"credential_ref":{"key_id":"key-1","token_id":"token-1"}}"#,
    )
    .expect("a principal")
}

fn alice_scopes() -> Vec<String> {
    [
        "alice",
        "user:alice",
        "team:team-1",
        "org:org-1",
        "key:key-1",
        "key:token-1",
    ]
    .map(str::to_owned)
    .to_vec()
}

fn skill_ids(outcome: &SkillSearchOutcome) -> Vec<&str> {
    match outcome {
        SkillSearchOutcome::Hits(hits) => {
            hits.iter().map(|hit| hit.skill.skill_id.as_str()).collect()
        }
        other => panic!("expected hits, got {other:?}"),
    }
}

fn scores(outcome: &SkillSearchOutcome) -> Vec<f64> {
    match outcome {
        SkillSearchOutcome::Hits(hits) => hits.iter().map(|hit| hit.score).collect(),
        other => panic!("expected hits, got {other:?}"),
    }
}

fn unsupported() -> SkillSearchOutcome {
    SkillSearchOutcome::UnsupportedProvider {
        reason: "query is only supported for custom_llm_provider=litellm_proxy".to_owned(),
    }
}

#[test]
fn constants_match_the_route_defaults() {
    assert_eq!(DEFAULT_SKILL_SEARCH_TOP_K, 5);
    assert_eq!(MAX_SKILL_SEARCH_TOP_K, 100);
    assert_eq!(MAX_SKILL_SEARCH_TEXT_CHARS, 4000);
}

#[test]
fn search_text_joins_the_present_parts() {
    let row = skill_row("litellm_skill_a", Some("Alpha"), Some(""));
    let mut with_instructions = row.clone();
    with_instructions.instructions = Some("Do it".to_owned());
    assert_eq!(skill_search_text(&with_instructions), "Alpha\nDo it");
    assert_eq!(skill_search_text(&row), "Alpha");
    let empty = skill_row("litellm_skill_b", None, None);
    assert_eq!(skill_search_text(&empty), "");
}

#[test]
fn search_text_cuts_at_the_unicode_character_limit() {
    let mut exact = skill_row("litellm_skill_a", None, None);
    exact.instructions = Some("é".repeat(4000));
    assert_eq!(skill_search_text(&exact).chars().count(), 4000);
    let mut over = skill_row("litellm_skill_b", None, None);
    over.instructions = Some("é".repeat(4001));
    let cut = skill_search_text(&over);
    assert_eq!(cut.chars().count(), 4000);
    assert_eq!(cut, "é".repeat(4000));
}

#[test]
fn rank_orders_by_score_and_keeps_ties() {
    let outcome = ranked(&FixedEmbedder::new(ranked_vectors()), 10);
    assert_eq!(
        skill_ids(&outcome),
        vec![
            "litellm_skill_d",
            "litellm_skill_a",
            "litellm_skill_b",
            "litellm_skill_c"
        ]
    );
    assert_eq!(
        scores(&outcome)
            .into_iter()
            .map(f64::to_bits)
            .collect::<Vec<u64>>(),
        [
            0.9486832980505138_f64,
            0.7071067811865475,
            0.7071067811865475,
            0.31622776601683794,
        ]
        .map(f64::to_bits)
        .to_vec()
    );
}

#[test]
fn rank_applies_the_callers_slice() {
    let embedder = FixedEmbedder::new(ranked_vectors());
    let first = ranked(&embedder, 1);
    assert_eq!(skill_ids(&first), vec!["litellm_skill_d"]);
    let none = ranked(&embedder, 0);
    assert_eq!(skill_ids(&none), Vec::<&str>::new());
    let all = ranked(&embedder, 10);
    assert_eq!(skill_ids(&all).len(), 4);
}

#[test]
fn rank_reports_an_embedding_failure() {
    let embedder = FixedEmbedder::failing(Failure::Value("boom".to_owned()));
    let outcome = rank_in(
        &SkillSearchIndex::new(10),
        &embedder,
        &ranked_skills(),
        10,
        MODEL,
    )
    .expect("a failed embedding is an outcome");
    assert_eq!(
        outcome,
        SkillSearchOutcome::EmbeddingFailed {
            reason: "embedding the search query failed: boom".to_owned(),
        }
    );
}

#[test]
fn rank_propagates_an_http_failure() {
    let embedder = FixedEmbedder::failing(Failure::Http {
        status_code: 429,
        detail: "slow down".to_owned(),
    });
    let error = rank_in(
        &SkillSearchIndex::new(10),
        &embedder,
        &ranked_skills(),
        10,
        MODEL,
    )
    .expect_err("an HTTP embedding failure propagates");
    assert!(matches!(
        error,
        SkillSearchError::Http {
            status_code: 429,
            detail,
        } if detail == "slow down"
    ));
}

#[test]
fn rank_rejects_a_non_finite_score() {
    let mut embedder = FixedEmbedder::new(vectors(&[("q", [1e308, 1e308])]));
    embedder
        .vectors
        .insert("Alpha".to_owned(), vec![1e308, 1e308]);
    let outcome = rank_in(
        &SkillSearchIndex::new(10),
        &embedder,
        &[skill_row("litellm_skill_a", Some("Alpha"), None)],
        10,
        MODEL,
    )
    .expect("a non-finite score is an outcome");
    assert_eq!(
        outcome,
        SkillSearchOutcome::EmbeddingFailed {
            reason: "embedding model test-embedding returned a score that is not a finite number"
                .to_owned(),
        }
    );
}

#[test]
fn rank_projects_the_skill_fields() {
    let hit = SkillSearchHit {
        skill: skill_row("litellm_skill_a", Some("Alpha"), None),
        score: 0.5,
    };
    let result = skill_search_result(&hit);
    assert_eq!(result.skill_id, "litellm_skill_a");
    assert_eq!(result.display_title.as_deref(), Some("Alpha"));
    assert_eq!(result.description, None);
    assert_eq!(result.score.to_bits(), 0.5_f64.to_bits());
    assert_eq!(
        serde_json::to_string(&result).expect("a projected result"),
        r#"{"skill_id":"litellm_skill_a","display_title":"Alpha","description":null,"score":0.5}"#
    );
}

#[test]
fn score_json_keeps_the_float_type() {
    let hit = SkillSearchHit {
        skill: skill_row("litellm_skill_a", Some("Alpha"), None),
        score: 1.0,
    };
    assert_eq!(
        serde_json::to_string(&skill_search_result(&hit)).expect("a projected result"),
        r#"{"skill_id":"litellm_skill_a","display_title":"Alpha","description":null,"score":1.0}"#
    );
}

#[test]
fn model_is_checked_before_the_router() {
    let embedder = FixedEmbedder::new(ranked_vectors());
    let index = SkillSearchIndex::new(10);
    let without_either = search_outcome(&index, None, None, &[]);
    assert_eq!(
        without_either,
        SkillSearchOutcome::NotConfigured {
            reason: MODEL_MISSING.to_owned(),
        }
    );
    let without_router = search_outcome(&index, None, Some(MODEL), &[]);
    assert_eq!(
        without_router,
        SkillSearchOutcome::NotConfigured {
            reason: ROUTER_MISSING.to_owned(),
        }
    );
    let model_missing = search_outcome(&index, Some(&embedder), None, &[]);
    assert_eq!(
        model_missing,
        SkillSearchOutcome::NotConfigured {
            reason: MODEL_MISSING.to_owned(),
        }
    );
    assert!(embedder.calls().is_empty());
}

#[test]
fn search_skills_embeds_through_the_injected_embedder() {
    let embedder = FixedEmbedder::new(vectors(&[("q", [1.0, 0.0]), ("Alpha", [1.0, 1.0])]));
    let skills = vec![skill_row("litellm_skill_a", Some("Alpha"), None)];
    let outcome = search_outcome(
        &SkillSearchIndex::new(10),
        Some(&embedder),
        Some(MODEL),
        &skills,
    );
    assert_eq!(skill_ids(&outcome), vec!["litellm_skill_a"]);
    assert_eq!(
        embedder.calls(),
        vec![vec!["q".to_owned(), "Alpha".to_owned()]]
    );
}

#[test]
fn empty_skills_skip_the_embedder() {
    let embedder = FixedEmbedder::new(ranked_vectors());
    let outcome =
        rank_in(&SkillSearchIndex::new(10), &embedder, &[], 10, MODEL).expect("an empty search");
    assert_eq!(outcome, SkillSearchOutcome::Hits(Vec::new()));
    assert!(embedder.calls().is_empty());
}

#[test]
fn hosted_search_rejects_other_providers() {
    let repository = ScriptedRepository::with_page(ranked_skills());
    let store = store_of(repository.clone());
    let embedder = FixedEmbedder::new(ranked_vectors());
    let index = SkillSearchIndex::new(10);
    let remote = hosted_outcome(&store, None, Some("anthropic"), &embedder, &index);
    let background = hosted_outcome(&store, None, None, &embedder, &index);
    assert_eq!(remote.expect("an outcome"), unsupported());
    assert_eq!(background.expect("an outcome"), unsupported());
    assert!(repository.effects().is_empty());
    assert!(embedder.calls().is_empty());
}

#[test]
fn hosted_search_ranks_the_store_page() {
    let repository = ScriptedRepository::with_page(vec![
        skill_row("litellm_skill_a", Some("Alpha"), None),
        skill_row("litellm_skill_b", Some("Beta"), None),
    ]);
    let store = store_of(repository.clone());
    let embedder = FixedEmbedder::new(vectors(&[
        ("q", [1.0, 0.0]),
        ("Alpha", [1.0, 1.0]),
        ("Beta", [1.0, 3.0]),
    ]));
    let index = SkillSearchIndex::new(10);
    let owner = principal();
    let outcome = hosted_outcome(
        &store,
        Some(&owner),
        Some("litellm_proxy"),
        &embedder,
        &index,
    )
    .expect("a ranked hosted search");
    assert_eq!(
        skill_ids(&outcome),
        vec!["litellm_skill_a", "litellm_skill_b"]
    );
    assert_eq!(repository.effects(), store_page_effects());
    assert_eq!(
        embedder.calls(),
        vec![vec!["q".to_owned(), "Alpha".to_owned(), "Beta".to_owned()]]
    );
}

fn store_page_effects() -> Vec<Effect> {
    vec![
        Effect::Availability,
        Effect::FindMany {
            page: SkillPage {
                limit: 5000,
                offset: 0,
            },
            created_by: Some(alice_scopes()),
        },
    ]
}

#[test]
fn hosted_search_reports_a_store_failure() {
    let store = store_of(ScriptedRepository::failing_page("the page read failed"));
    let embedder = FixedEmbedder::new(ranked_vectors());
    let index = SkillSearchIndex::new(10);
    let outcome = hosted_outcome(&store, None, Some("litellm_proxy"), &embedder, &index);
    assert!(matches!(outcome, Err(HostedSkillSearchError::Store(_))));
    assert!(embedder.calls().is_empty());
}

#[test]
fn repeat_query() {
    let embedder = FixedEmbedder::new(vectors(&[
        ("q", [1.0, 0.0]),
        ("Alpha", [1.0, 1.0]),
        ("Beta", [1.0, 3.0]),
    ]));
    let index = SkillSearchIndex::new(10);
    let skills = vec![
        skill_row("litellm_skill_a", Some("Alpha"), None),
        skill_row("litellm_skill_b", Some("Beta"), None),
    ];
    rank_in(&index, &embedder, &skills, 10, MODEL).expect("a ranked search");
    rank_in(&index, &embedder, &skills, 10, MODEL).expect("a ranked search");
    assert_eq!(
        embedder.calls(),
        vec![
            vec!["q".to_owned(), "Alpha".to_owned(), "Beta".to_owned()],
            vec!["q".to_owned()],
        ]
    );
}

#[test]
fn model_change() {
    let embedder = FixedEmbedder::new(vectors(&[("q", [1.0, 0.0]), ("Alpha", [1.0, 1.0])]));
    let index = SkillSearchIndex::new(10);
    let skills = vec![skill_row("litellm_skill_a", Some("Alpha"), None)];
    rank_in(&index, &embedder, &skills, 10, "first-embedding").expect("a ranked search");
    rank_in(&index, &embedder, &skills, 10, "second-embedding").expect("a ranked search");
    assert_eq!(
        embedder.calls(),
        vec![
            vec!["q".to_owned(), "Alpha".to_owned()],
            vec!["q".to_owned(), "Alpha".to_owned()],
        ]
    );
}

#[test]
fn small_index_evicts_the_oldest_text() {
    let embedder = FixedEmbedder::new(vectors(&[
        ("q", [1.0, 0.0]),
        ("Alpha", [1.0, 1.0]),
        ("Beta", [1.0, 3.0]),
    ]));
    let index = SkillSearchIndex::new(1);
    let skills = vec![
        skill_row("litellm_skill_a", Some("Alpha"), None),
        skill_row("litellm_skill_b", Some("Beta"), None),
    ];
    rank_in(&index, &embedder, &skills, 10, MODEL).expect("a ranked search");
    rank_in(&index, &embedder, &skills, 10, MODEL).expect("a ranked search");
    assert_eq!(
        embedder.calls(),
        vec![
            vec!["q".to_owned(), "Alpha".to_owned(), "Beta".to_owned()],
            vec!["q".to_owned(), "Alpha".to_owned()],
        ]
    );
}

#[test]
fn default_index_keeps_the_candidate_limit() {
    let skills = (0..5001)
        .map(|index| {
            serde_json::from_value(json!({
                "skill_id": format!("litellm_skill_{index}"),
                "display_title": format!("skill-{index}"),
            }))
            .expect("a scripted skill row")
        })
        .collect::<Vec<SkillTable>>();
    let mut text_vectors = (0..5001)
        .map(|index| (format!("skill-{index}"), vec![1.0_f64, 1.0]))
        .collect::<BTreeMap<String, Vector>>();
    text_vectors.insert("q".to_owned(), vec![1.0_f64, 0.0]);
    let embedder = FixedEmbedder::new(text_vectors);
    let index = SkillSearchIndex::default();
    rank_in(&index, &embedder, &skills, 10, MODEL).expect("a ranked search");
    rank_in(&index, &embedder, &skills, 10, MODEL).expect("a ranked search");
    let calls = embedder.calls();
    assert_eq!(calls[0].len(), 5002);
    assert_eq!(calls[1], vec!["q".to_owned(), "skill-0".to_owned()]);
}
