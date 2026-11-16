#![cfg(test)]

mod postgres;

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_context::dynamic_logging_cache::Clock;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::skills::SkillTable;
use betterlite_db::managed_resource_rows::wire::RowTimestamp;
use betterlite_db::postgres_store::errors::DbError;

use super::StoredSkillReads;
use super::cache::SkillCacheLimits;
use super::error::StoredSkillReadError;
use super::stored_skill::StoredSkill;
use crate::skills::store::SkillPage;
use crate::skills::store::repository::{SkillRepository, SkillWrite};

const ALICE_SKILL: &str = "litellm_skill_alice";

fn timestamp(text: &str) -> RowTimestamp {
    serde_json::from_str(&format!("\"{text}\"")).unwrap()
}

fn row(skill_id: &str, owner: Option<&str>) -> SkillTable {
    SkillTable {
        skill_id: skill_id.to_owned(),
        display_title: Some(format!("Title {skill_id}")),
        description: None,
        instructions: None,
        source: "custom".to_owned(),
        latest_version: None,
        file_content: None,
        file_name: None,
        file_type: None,
        metadata: None,
        created_at: Some(timestamp("2026-01-01T00:00:00.123456")),
        created_by: owner.map(str::to_owned),
        updated_at: Some(timestamp("2026-01-01T00:00:00.123456")),
        updated_by: owner.map(str::to_owned),
    }
}

fn row_with_content(skill_id: &str, owner: Option<&str>, content: &str) -> SkillTable {
    let mut skill = row(skill_id, owner);
    skill.file_content = Some(content.to_owned());
    skill
}

fn principal(user: &str) -> Principal {
    let document = format!(
        r#"{{"principal_type":"human","subject":"{user}","auth_method":"api_key","user":{{"id":"{user}"}},"teams":[{{"id":"team-1"}}],"organization":{{"id":"org-1"}},"credential_ref":{{"key_id":"key-1","token_id":"tok-1"}}}}"#
    );
    serde_json::from_str(&document).unwrap()
}

fn admin(user: &str) -> Principal {
    let mut identity = principal(user);
    identity.roles = vec![Role::PlatformAdmin];
    identity
}

fn anonymous() -> Principal {
    serde_json::from_str(r#"{"principal_type":"human","subject":"anon","auth_method":"api_key"}"#)
        .unwrap()
}

fn fixed_clock() -> Clock {
    Arc::new(|| 1_000.0_f64)
}

fn movable_clock() -> (Clock, Arc<Mutex<f64>>) {
    let value = Arc::new(Mutex::new(1_000.0_f64));
    let reading = Arc::clone(&value);
    let clock: Clock = Arc::new(move || *reading.lock().unwrap_or_else(PoisonError::into_inner));
    (clock, value)
}

fn set_clock(clock: &Mutex<f64>, value: f64) {
    *clock.lock().unwrap_or_else(PoisonError::into_inner) = value;
}

fn service(repository: ScriptedRepository) -> StoredSkillReads<ScriptedRepository> {
    service_with(repository, SkillCacheLimits::default(), fixed_clock())
}

fn service_with(
    repository: ScriptedRepository,
    limits: SkillCacheLimits,
    clock: Clock,
) -> StoredSkillReads<ScriptedRepository> {
    StoredSkillReads::new(repository, limits, clock).unwrap()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    FindUnique(String),
    FindMany {
        page: SkillPage,
        created_by: Option<Vec<String>>,
    },
}

struct State {
    available: bool,
    failed: bool,
    rows: BTreeMap<String, SkillTable>,
    page: Vec<SkillTable>,
    calls: Vec<Call>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            available: true,
            failed: false,
            rows: BTreeMap::new(),
            page: Vec::new(),
            calls: Vec::new(),
        }
    }
}

#[derive(Clone, Default)]
struct ScriptedRepository {
    state: Arc<Mutex<State>>,
}

impl ScriptedRepository {
    fn with_rows(rows: Vec<SkillTable>) -> Self {
        let repository = Self::default();
        let mut state = repository
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for row in rows {
            state.rows.insert(row.skill_id.clone(), row.clone());
            state.page.push(row);
        }
        drop(state);
        repository
    }

    fn set_available(&self, available: bool) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .available = available;
    }

    fn set_failed(&self, failed: bool) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .failed = failed;
    }

    fn replace_row(&self, row: SkillTable) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .rows
            .insert(row.skill_id.clone(), row);
    }

    fn calls(&self) -> Vec<Call> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .calls
            .clone()
    }

    fn find_unique_calls(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::FindUnique(skill_id) => Some(skill_id),
                Call::FindMany { .. } => None,
            })
            .collect()
    }

    fn find_many_calls(&self) -> Vec<(SkillPage, Option<Vec<String>>)> {
        let mut pages = Vec::new();
        for call in self.calls() {
            match call {
                Call::FindMany { page, created_by } => pages.push((page, created_by)),
                Call::FindUnique(_) => {}
            }
        }
        pages
    }

    fn found_row(&self, skill_id: &str) -> Result<Option<SkillTable>, TableError> {
        let (row, failed) = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.calls.push(Call::FindUnique(skill_id.to_owned()));
            (state.rows.get(skill_id).cloned(), state.failed)
        };
        if failed {
            return Err(TableError::Publish {
                message: "the scripted repository failed the read".to_owned(),
            });
        }
        Ok(row)
    }

    fn found_page(
        &self,
        page: SkillPage,
        created_by: Option<Vec<String>>,
    ) -> Result<Vec<SkillTable>, TableError> {
        let (rows, failed) = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.calls.push(Call::FindMany {
                page,
                created_by: created_by.clone(),
            });
            (state.page.clone(), state.failed)
        };
        if failed {
            return Err(TableError::Publish {
                message: "the scripted repository failed the read".to_owned(),
            });
        }
        let offset = usize::try_from(page.offset).unwrap_or(usize::MAX);
        let limit = usize::try_from(page.limit).unwrap_or(0);
        rows.into_iter()
            .filter(|row| match &created_by {
                Some(scopes) => row
                    .created_by
                    .as_ref()
                    .is_some_and(|owner| scopes.contains(owner)),
                None => true,
            })
            .skip(offset)
            .take(limit)
            .collect()
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
            message: "create is not scripted".to_owned(),
        }))
    }

    fn find_unique(
        &self,
        skill_id: &str,
    ) -> impl Future<Output = Result<Option<SkillTable>, TableError>> + Send {
        ready(self.found_row(skill_id))
    }

    fn find_many(
        &self,
        page: SkillPage,
        created_by: Option<Vec<String>>,
    ) -> impl Future<Output = Result<Vec<SkillTable>, TableError>> + Send {
        ready(self.found_page(page, created_by))
    }

    fn delete(&self, _skill_id: &str) -> impl Future<Output = Result<(), TableError>> + Send {
        ready(Err(TableError::Publish {
            message: "delete is not scripted".to_owned(),
        }))
    }
}

#[tokio::test]
async fn stored_skill_reads_missing_principal() {
    let repository = ScriptedRepository::with_rows(vec![
        row(ALICE_SKILL, Some("alice")),
        row("litellm_skill_bob", Some("bob")),
    ]);
    let reads = service(repository.clone());
    let public_page = reads
        .list_skills(
            SkillPage {
                limit: 1000,
                offset: 0,
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(public_page.len(), 2);
    assert_eq!(
        repository.find_many_calls(),
        vec![(
            SkillPage {
                limit: 1000,
                offset: 0,
            },
            None,
        )]
    );
    let skill = reads.get_skill(ALICE_SKILL, None).await.unwrap();
    assert_eq!(skill.skill_id, ALICE_SKILL);
    assert_eq!(
        repository.calls(),
        vec![
            Call::FindMany {
                page: SkillPage {
                    limit: 1000,
                    offset: 0,
                },
                created_by: None,
            },
            Call::FindUnique(ALICE_SKILL.to_owned()),
        ]
    );

    let identity = anonymous();
    let identityless_page = reads
        .list_skills(SkillPage::default(), Some(&identity))
        .await
        .unwrap();
    assert!(identityless_page.is_empty());
    let refused = reads
        .get_skill(ALICE_SKILL, Some(&identity))
        .await
        .unwrap_err();
    assert!(matches!(refused, StoredSkillReadError::NotFound { .. }));
    assert_eq!(repository.find_unique_calls(), vec![ALICE_SKILL.to_owned()]);
    assert_eq!(repository.find_many_calls().len(), 1);
}

#[tokio::test]
async fn list_skills_scopes_the_page_to_the_callers_owner_scopes() {
    let repository = ScriptedRepository::with_rows(vec![
        row("litellm_skill_alice", Some("alice")),
        row("litellm_skill_bob", Some("bob")),
        row("litellm_skill_team", Some("team:team-1")),
    ]);
    let reads = service(repository.clone());
    let listed = reads
        .list_skills(
            SkillPage {
                limit: 20,
                offset: 0,
            },
            Some(&principal("alice")),
        )
        .await
        .unwrap();
    let ids = listed
        .iter()
        .map(|skill| skill.skill_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["litellm_skill_alice", "litellm_skill_team"]);
    assert_eq!(
        repository.find_many_calls(),
        vec![(
            SkillPage {
                limit: 20,
                offset: 0,
            },
            Some(
                [
                    "alice",
                    "user:alice",
                    "team:team-1",
                    "org:org-1",
                    "key:key-1",
                    "key:tok-1",
                ]
                .map(str::to_owned)
                .to_vec()
            ),
        )]
    );
}

#[tokio::test]
async fn list_skills_admin_and_null_principal_skip_the_owner_filter() {
    let repository = ScriptedRepository::with_rows(vec![
        row("litellm_skill_alice", Some("alice")),
        row("litellm_skill_bob", Some("bob")),
    ]);
    let reads = service(repository.clone());

    let admin_page = reads
        .list_skills(SkillPage::default(), Some(&admin("root")))
        .await
        .unwrap();
    assert_eq!(admin_page.len(), 2);
    let public_page = reads.list_skills(SkillPage::default(), None).await.unwrap();
    assert_eq!(public_page.len(), 2);
    assert_eq!(
        repository.find_many_calls(),
        vec![(SkillPage::default(), None), (SkillPage::default(), None),]
    );
}

#[tokio::test]
async fn list_skills_without_owner_scopes_returns_empty_without_a_query() {
    let repository = ScriptedRepository::with_rows(vec![row(ALICE_SKILL, Some("alice"))]);
    let reads = service(repository.clone());
    let listed = reads
        .list_skills(SkillPage::default(), Some(&anonymous()))
        .await
        .unwrap();
    assert!(listed.is_empty());
    assert!(repository.calls().is_empty());
}

#[tokio::test]
async fn list_skills_passes_the_page_to_the_repository() {
    let repository = ScriptedRepository::with_rows(vec![
        row("litellm_skill_newest", Some("alice")),
        row("litellm_skill_middle", Some("alice")),
        row("litellm_skill_oldest", Some("alice")),
    ]);
    let reads = service(repository.clone());
    let listed = reads
        .list_skills(
            SkillPage {
                limit: 1,
                offset: 1,
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].skill_id, "litellm_skill_middle");
    assert_eq!(
        repository.find_many_calls(),
        vec![(
            SkillPage {
                limit: 1,
                offset: 1,
            },
            None,
        )]
    );
}

#[test]
fn skill_to_typed_conversion_decodes_base64_file_content() {
    let mut skill = row_with_content(ALICE_SKILL, Some("alice"), "aGVs\nbG8=");
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "tier".to_owned(),
        JsonData::Number(serde_json::Number::from(2_i32)),
    );
    skill.metadata = Some(metadata);
    skill.latest_version = Some("2".to_owned());
    let converted = StoredSkill::from_row(skill).unwrap();
    assert_eq!(converted.skill_id, ALICE_SKILL);
    assert_eq!(converted.file_content, Some(b"hello".to_vec()));
    assert_eq!(converted.created_by.as_deref(), Some("alice"));
    assert_eq!(converted.latest_version.as_deref(), Some("2"));
    assert_eq!(
        converted
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("tier")),
        Some(&JsonData::Number(serde_json::Number::from(2_i32)))
    );
    assert_eq!(
        converted.created_at,
        Some(timestamp("2026-01-01T00:00:00.123456"))
    );
}

#[test]
fn skill_to_typed_conversion_keeps_absent_and_empty_content() {
    let absent = StoredSkill::from_row(row(ALICE_SKILL, Some("alice"))).unwrap();
    assert_eq!(absent.file_content, None);
    let empty = StoredSkill::from_row(row_with_content("litellm_skill_empty", None, "")).unwrap();
    assert_eq!(empty.file_content, Some(Vec::new()));
}

#[test]
fn skill_to_typed_conversion_rejects_malformed_base64() {
    let padding =
        StoredSkill::from_row(row_with_content(ALICE_SKILL, Some("alice"), "abc")).unwrap_err();
    assert!(matches!(padding, StoredSkillReadError::FileContent(_)));
    let alphabet =
        StoredSkill::from_row(row_with_content(ALICE_SKILL, Some("alice"), "ab-_")).unwrap_err();
    assert!(matches!(alphabet, StoredSkillReadError::FileContent(_)));
}

#[tokio::test]
async fn get_skill_decodes_the_owned_row() {
    let repository =
        ScriptedRepository::with_rows(vec![row_with_content(ALICE_SKILL, Some("alice"), "emlw")]);
    let reads = service(repository);
    let skill = reads
        .get_skill(ALICE_SKILL, Some(&principal("alice")))
        .await
        .unwrap();
    assert_eq!(skill.skill_id, ALICE_SKILL);
    assert_eq!(skill.file_content, Some(b"zip".to_vec()));
}

#[tokio::test]
async fn get_skill_returns_one_not_found_for_missing_and_cross_owner_rows() {
    let repository = ScriptedRepository::with_rows(vec![row(ALICE_SKILL, Some("alice"))]);
    let reads = service(repository.clone());
    let denied = reads
        .get_skill(ALICE_SKILL, Some(&principal("bob")))
        .await
        .unwrap_err();
    assert!(matches!(denied, StoredSkillReadError::NotFound { .. }));

    let empty = service(ScriptedRepository::default());
    let missing = empty
        .get_skill(ALICE_SKILL, Some(&principal("alice")))
        .await
        .unwrap_err();
    assert!(matches!(missing, StoredSkillReadError::NotFound { .. }));
    assert_eq!(denied.to_string(), missing.to_string());
    assert_eq!(repository.find_unique_calls(), vec![ALICE_SKILL.to_owned()]);
}

#[tokio::test]
async fn cache_serves_every_caller_from_one_per_skill_entry() {
    let repository = ScriptedRepository::with_rows(vec![row(ALICE_SKILL, Some("alice"))]);
    let reads = service(repository.clone());
    let owner = reads
        .get_skill(ALICE_SKILL, Some(&principal("alice")))
        .await
        .unwrap();
    assert_eq!(
        owner.display_title.as_deref(),
        Some("Title litellm_skill_alice")
    );
    let refused = reads
        .get_skill(ALICE_SKILL, Some(&principal("bob")))
        .await
        .unwrap_err();
    assert!(matches!(refused, StoredSkillReadError::NotFound { .. }));
    assert!(
        reads
            .get_skill(ALICE_SKILL, Some(&admin("root")))
            .await
            .is_ok()
    );
    assert!(reads.get_skill(ALICE_SKILL, None).await.is_ok());
    assert_eq!(repository.find_unique_calls(), vec![ALICE_SKILL.to_owned()]);
}

#[tokio::test]
async fn cache_remembers_missing_skills_until_the_default_ttl_passes() {
    let repository = ScriptedRepository::default();
    let (clock, now) = movable_clock();
    let reads = service_with(repository.clone(), SkillCacheLimits::default(), clock);
    let ghost = "litellm_skill_ghost";
    assert!(matches!(
        reads
            .get_skill(ghost, Some(&principal("alice")))
            .await
            .unwrap_err(),
        StoredSkillReadError::NotFound { .. }
    ));
    set_clock(&now, 1_059.0_f64);
    assert!(matches!(
        reads
            .get_skill(ghost, Some(&principal("alice")))
            .await
            .unwrap_err(),
        StoredSkillReadError::NotFound { .. }
    ));
    assert_eq!(repository.find_unique_calls(), vec![ghost.to_owned()]);

    repository.replace_row(row(ghost, Some("alice")));
    set_clock(&now, 1_061.0_f64);
    let found = reads
        .get_skill(ghost, Some(&principal("alice")))
        .await
        .unwrap();
    assert_eq!(found.skill_id, ghost);
    assert_eq!(
        repository.find_unique_calls(),
        vec![ghost.to_owned(), ghost.to_owned()]
    );
}

#[tokio::test]
async fn cache_expires_a_positive_row_after_the_default_ttl() {
    let repository = ScriptedRepository::with_rows(vec![row(ALICE_SKILL, Some("alice"))]);
    let (clock, now) = movable_clock();
    let reads = service_with(repository.clone(), SkillCacheLimits::default(), clock);
    let stale = reads
        .get_skill(ALICE_SKILL, Some(&principal("alice")))
        .await
        .unwrap();
    assert_eq!(
        stale.display_title.as_deref(),
        Some("Title litellm_skill_alice")
    );

    let mut changed = row(ALICE_SKILL, Some("alice"));
    changed.display_title = Some("Changed".to_owned());
    repository.replace_row(changed);
    set_clock(&now, 1_059.0_f64);
    let cached = reads
        .get_skill(ALICE_SKILL, Some(&principal("alice")))
        .await
        .unwrap();
    assert_eq!(
        cached.display_title.as_deref(),
        Some("Title litellm_skill_alice")
    );

    set_clock(&now, 1_061.0_f64);
    let refreshed = reads
        .get_skill(ALICE_SKILL, Some(&principal("alice")))
        .await
        .unwrap();
    assert_eq!(refreshed.display_title.as_deref(), Some("Changed"));
    assert_eq!(
        repository.find_unique_calls(),
        vec![ALICE_SKILL.to_owned(), ALICE_SKILL.to_owned()]
    );
}

#[tokio::test]
async fn cache_uses_the_configured_limits() {
    let repository = ScriptedRepository::with_rows(vec![row(ALICE_SKILL, Some("alice"))]);
    let (clock, now) = movable_clock();
    let limits = SkillCacheLimits {
        max_size_in_memory: 4,
        default_ttl: 5.0_f64,
    };
    let reads = service_with(repository.clone(), limits, clock);
    assert!(
        reads
            .get_skill(ALICE_SKILL, Some(&principal("alice")))
            .await
            .is_ok()
    );
    set_clock(&now, 1_004.0_f64);
    assert!(
        reads
            .get_skill(ALICE_SKILL, Some(&principal("alice")))
            .await
            .is_ok()
    );
    assert_eq!(repository.find_unique_calls(), vec![ALICE_SKILL.to_owned()]);
    set_clock(&now, 1_006.0_f64);
    assert!(
        reads
            .get_skill(ALICE_SKILL, Some(&principal("alice")))
            .await
            .is_ok()
    );
    assert_eq!(
        repository.find_unique_calls(),
        vec![ALICE_SKILL.to_owned(), ALICE_SKILL.to_owned()]
    );
}

#[tokio::test]
async fn cache_capacity_drops_an_entry_once_the_limit_is_reached() {
    let other = "litellm_skill_other";
    let repository = ScriptedRepository::with_rows(vec![
        row(ALICE_SKILL, Some("alice")),
        row(other, Some("alice")),
    ]);
    let limits = SkillCacheLimits {
        max_size_in_memory: 1,
        default_ttl: 60.0_f64,
    };
    let reads = service_with(repository.clone(), limits, fixed_clock());
    assert!(
        reads
            .get_skill(ALICE_SKILL, Some(&principal("alice")))
            .await
            .is_ok()
    );
    assert!(
        reads
            .get_skill(other, Some(&principal("alice")))
            .await
            .is_ok()
    );
    assert!(
        reads
            .get_skill(ALICE_SKILL, Some(&principal("alice")))
            .await
            .is_ok()
    );
    assert_eq!(
        repository.find_unique_calls(),
        vec![
            ALICE_SKILL.to_owned(),
            other.to_owned(),
            ALICE_SKILL.to_owned()
        ]
    );
}

#[tokio::test]
async fn load_skill_answers_from_the_cache_while_the_database_is_down() {
    let repository = ScriptedRepository::with_rows(vec![row(ALICE_SKILL, Some("alice"))]);
    let reads = service(repository.clone());
    assert!(
        reads
            .get_skill(ALICE_SKILL, Some(&principal("alice")))
            .await
            .is_ok()
    );
    repository.set_available(false);
    assert!(
        reads
            .get_skill(ALICE_SKILL, Some(&principal("alice")))
            .await
            .is_ok()
    );
    assert!(matches!(
        reads
            .get_skill(ALICE_SKILL, Some(&principal("bob")))
            .await
            .unwrap_err(),
        StoredSkillReadError::NotFound { .. }
    ));
    assert!(matches!(
        reads
            .get_skill("litellm_skill_ghost", Some(&principal("alice")))
            .await
            .unwrap_err(),
        StoredSkillReadError::NoDatabase
    ));
    assert!(matches!(
        reads
            .list_skills(SkillPage::default(), Some(&principal("alice")))
            .await
            .unwrap_err(),
        StoredSkillReadError::NoDatabase
    ));
    assert_eq!(repository.find_unique_calls(), vec![ALICE_SKILL.to_owned()]);
}

#[tokio::test]
async fn reads_report_the_repository_failure() {
    let repository = ScriptedRepository::with_rows(vec![row(ALICE_SKILL, Some("alice"))]);
    repository.set_failed(true);
    let reads = service(repository);
    assert!(matches!(
        reads
            .get_skill(ALICE_SKILL, Some(&principal("alice")))
            .await
            .unwrap_err(),
        StoredSkillReadError::Repository { .. }
    ));
    assert!(matches!(
        reads
            .list_skills(SkillPage::default(), None)
            .await
            .unwrap_err(),
        StoredSkillReadError::Repository { .. }
    ));
}

#[test]
fn new_rejects_a_negative_cache_capacity() {
    let limits = SkillCacheLimits {
        max_size_in_memory: -1,
        default_ttl: 60.0_f64,
    };
    let result = StoredSkillReads::new(ScriptedRepository::default(), limits, fixed_clock());
    assert!(matches!(result, Err(StoredSkillReadError::Cache(_))));
}
