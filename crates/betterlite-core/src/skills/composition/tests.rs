#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::dynamic_logging_cache::Clock;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::managed_resource_rows::skills::SkillTable;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_semantic::{EmbedError, Embedder, Vector};
use tokio::runtime::{Builder, Runtime};

use super::{HostedSkillSearch, HostedSkills};
use crate::skills::sandbox::executor::SandboxExecutor;
use crate::skills::sandbox::session::{
    SandboxSession, SessionError, SessionOpener, SessionOptions,
};
use crate::skills::search::{HostedSkillSearchError, SkillSearchOutcome};
use crate::skills::store::repository::{SkillRepository, SkillWrite};
use crate::skills::store::{
    DEFAULT_PAGE_OFFSET, MAX_SKILLS_PER_SEARCH, SkillIdSource, SkillPage, StoreError,
};

struct RepositoryState {
    available: bool,
    page: Vec<SkillTable>,
    asked: Option<(SkillPage, Option<Vec<String>>)>,
}

impl Default for RepositoryState {
    fn default() -> Self {
        Self {
            available: true,
            page: Vec::new(),
            asked: None,
        }
    }
}

#[derive(Clone, Default)]
struct ScriptedRepository {
    state: Arc<Mutex<RepositoryState>>,
}

impl ScriptedRepository {
    fn with_page(page: Vec<SkillTable>) -> Self {
        let repository = Self::default();
        repository
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .page = page;
        repository
    }

    fn unavailable() -> Self {
        let repository = Self::default();
        repository
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .available = false;
        repository
    }

    fn asked(&self) -> Option<(SkillPage, Option<Vec<String>>)> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .asked
            .clone()
    }
}

impl SkillRepository for ScriptedRepository {
    fn availability(&self) -> Result<(), TableError> {
        let available = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .available;
        if available {
            return Ok(());
        }
        Err(TableError::Database(DbError::NoDbConnected))
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
        ready(Ok(None))
    }

    fn find_many(
        &self,
        page: SkillPage,
        created_by: Option<Vec<String>>,
    ) -> impl Future<Output = Result<Vec<SkillTable>, TableError>> + Send {
        let rows = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.asked = Some((page, created_by));
            state.page.clone()
        };
        ready(Ok(rows))
    }

    fn delete(&self, _skill_id: &str) -> impl Future<Output = Result<(), TableError>> + Send {
        ready(Err(TableError::Publish {
            message: "the scripted repository has no delete".to_owned(),
        }))
    }
}

struct FixedEmbedder {
    vectors: BTreeMap<String, Vector>,
}

impl FixedEmbedder {
    fn of(items: &[(&str, [f64; 2])]) -> Self {
        Self {
            vectors: items
                .iter()
                .map(|(text, vector)| ((*text).to_owned(), vector.to_vec()))
                .collect(),
        }
    }
}

#[async_trait]
impl Embedder for FixedEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vector>, EmbedError> {
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

struct NoSession;

#[async_trait]
impl SessionOpener for NoSession {
    async fn open(
        &self,
        _options: &SessionOptions,
    ) -> Result<Box<dyn SandboxSession>, SessionError> {
        Err(SessionError::SdkMissing)
    }
}

fn runtime() -> Runtime {
    Builder::new_current_thread().build().expect("a runtime")
}

fn ids() -> SkillIdSource {
    Arc::new(|| String::from("00000000-0000-4000-8000-000000000000"))
}

fn clock() -> Clock {
    Arc::new(|| 1_000.0_f64)
}

fn sandbox() -> SandboxExecutor<NoSession> {
    SandboxExecutor {
        opener: NoSession,
        image: Some(String::from("python:3.12")),
        timeout: Duration::from_secs(7),
    }
}

fn skill_row() -> SkillTable {
    SkillTable {
        skill_id: String::from("litellm_skill_shared"),
        display_title: Some(String::from("Skill Title")),
        description: Some(String::from("Look up the weather")),
        instructions: None,
        source: String::from("custom"),
        latest_version: None,
        file_content: None,
        file_name: None,
        file_type: None,
        metadata: None,
        created_at: None,
        created_by: None,
        updated_at: None,
        updated_by: None,
    }
}

#[test]
fn the_search_reads_the_store_the_operations_share() {
    let repository = ScriptedRepository::with_page(vec![skill_row()]);
    let composition =
        HostedSkills::new(repository.clone(), ids(), clock(), sandbox()).expect("the composition");
    let embedder = FixedEmbedder::of(&[
        ("Skill Title\nLook up the weather", [1.0, 0.0]),
        ("Look up the weather", [1.0, 0.0]),
    ]);
    let outcome = runtime()
        .block_on(composition.search(
            None,
            HostedSkillSearch {
                custom_llm_provider: Some("litellm_proxy"),
                query: "Look up the weather",
                top_k: 5,
                embedder: Some(&embedder),
                embedding_model: Some("test-embedding"),
            },
        ))
        .expect("the search answers");
    let SkillSearchOutcome::Hits(hits) = outcome else {
        panic!("a configured search is not a hit list");
    };
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].skill.skill_id, "litellm_skill_shared");
    assert_eq!(
        repository.asked(),
        Some((
            SkillPage {
                limit: MAX_SKILLS_PER_SEARCH,
                offset: DEFAULT_PAGE_OFFSET,
            },
            None,
        ))
    );
}

#[test]
fn a_missing_database_answers_through_the_composed_store() {
    let composition =
        HostedSkills::new(ScriptedRepository::unavailable(), ids(), clock(), sandbox())
            .expect("the composition");
    let outcome = runtime().block_on(composition.search(
        None,
        HostedSkillSearch {
            custom_llm_provider: Some("litellm_proxy"),
            query: "anything",
            top_k: 5,
            embedder: None,
            embedding_model: None,
        },
    ));
    assert!(matches!(
        outcome,
        Err(HostedSkillSearchError::Store(StoreError::NoDatabase))
    ));
}

#[test]
fn the_sandbox_is_the_leaf_the_caller_built() {
    let composition = HostedSkills::new(ScriptedRepository::default(), ids(), clock(), sandbox())
        .expect("the composition");
    assert_eq!(composition.sandbox.timeout, Duration::from_secs(7));
    assert_eq!(composition.sandbox.image.as_deref(), Some("python:3.12"));
}
