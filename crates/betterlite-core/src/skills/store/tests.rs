#![cfg(test)]

mod postgres;
mod support;

use std::collections::BTreeMap;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::errors::DbError;

use super::repository::{PrismaSkillRepository, SkillRepository as _, SkillWrite};
use super::{
    HostedSkillStore, NEGATIVE_SKILL_SENTINEL, NewSkill, SKILL_ID_PREFIX, SkillPage, StoreError,
};

use support::{Effect, SKILL_ID, ScriptedRepository, admin, principal, stored_row};

fn store(repository: ScriptedRepository) -> HostedSkillStore<ScriptedRepository> {
    HostedSkillStore::new(repository, support::fixed_ids(), support::fixed_clock()).unwrap()
}

fn anonymous() -> Principal {
    serde_json::from_str(r#"{"principal_type":"human","subject":"anon","auth_method":"api_key"}"#)
        .unwrap()
}

fn new_skill() -> NewSkill {
    NewSkill {
        display_title: Some("Title".to_owned()),
        description: None,
        instructions: Some("Do".to_owned()),
        metadata: None,
        file_content: None,
        file_name: None,
        file_type: None,
    }
}

fn scopes() -> Vec<String> {
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

fn page_effects(repository: &ScriptedRepository) -> Vec<Effect> {
    repository.effects()
}

#[test]
fn create_stamps_the_primary_scope_and_encodes_the_row() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let store = store(repository.clone());
        let mut metadata = BTreeMap::new();
        metadata.insert("note".to_owned(), JsonData::String("a\0b".to_owned()));
        let data = NewSkill {
            display_title: Some("Title".to_owned()),
            description: None,
            instructions: Some("Do".to_owned()),
            metadata: Some(metadata),
            file_content: Some(b"zip".to_vec()),
            file_name: Some("skill.zip".to_owned()),
            file_type: Some("application/zip".to_owned()),
        };
        let row = store
            .create_skill(&data, Some("ignored"), Some(&principal("alice")))
            .await
            .unwrap();
        assert_eq!(row.skill_id, SKILL_ID);
        assert!(row.skill_id.starts_with(SKILL_ID_PREFIX));
        assert_eq!(row.source, "custom");
        assert_eq!(row.created_by.as_deref(), Some("alice"));
        assert_eq!(row.updated_by.as_deref(), Some("alice"));
        assert_eq!(row.file_content.as_deref(), Some("emlw"));
        assert_eq!(row.file_name.as_deref(), Some("skill.zip"));
        let effects = page_effects(&repository);
        let Effect::Create(write) = &effects[0] else {
            panic!("create must reach the repository once");
        };
        assert_eq!(write.skill_id, SKILL_ID);
        assert_eq!(write.source, "custom");
        assert_eq!(write.created_by, "alice");
        assert_eq!(write.updated_by, "alice");
        assert_eq!(write.metadata.as_deref(), Some("{\"note\":\"ab\"}"));
        assert_eq!(write.file_content.as_deref(), Some("emlw"));
        assert_eq!(write.file_type.as_deref(), Some("application/zip"));
    });
}

#[test]
fn create_falls_back_to_the_user_id() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let store = store(repository.clone());
        let row = store
            .create_skill(&new_skill(), Some("legacy"), Some(&anonymous()))
            .await
            .unwrap();
        assert_eq!(row.created_by.as_deref(), Some("legacy"));
        assert_eq!(row.updated_by.as_deref(), Some("legacy"));
    });
}

#[test]
fn create_uses_an_empty_user_id_as_the_owner_scope() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let store = store(repository.clone());
        let row = store
            .create_skill(&new_skill(), Some(""), None)
            .await
            .unwrap();
        assert_eq!(row.created_by.as_deref(), Some(""));
        assert_eq!(row.updated_by.as_deref(), Some(""));
    });
}

#[test]
fn create_rejects_a_caller_without_identity_before_writing() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let store = store(repository.clone());
        let error = store
            .create_skill(&new_skill(), None, None)
            .await
            .unwrap_err();
        assert!(matches!(error, StoreError::MissingIdentity));
        assert_eq!(
            error.to_string(),
            "Unable to record skill ownership: caller has no identity scope."
        );
        assert!(repository.effects().is_empty());
    });
}

#[test]
fn create_checks_the_database_before_the_identity() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        repository.update(|state| state.available = false);
        let store = store(repository.clone());
        let error = store
            .create_skill(&new_skill(), None, None)
            .await
            .unwrap_err();
        assert!(matches!(error, StoreError::NoDatabase));
        assert_eq!(
            error.to_string(),
            "Prisma client is not initialized. Database connection required for LiteLLM skills."
        );
        assert!(repository.effects().is_empty());
    });
}

#[test]
fn list_defaults_to_twenty_and_the_owner_scopes() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        repository.update(|state| state.page = vec![stored_row("alice")]);
        let store = store(repository.clone());
        let rows = store
            .list_skills(SkillPage::default(), Some(&principal("alice")))
            .await
            .unwrap();
        assert_eq!(rows, vec![stored_row("alice")]);
        assert_eq!(
            page_effects(&repository),
            vec![Effect::FindMany {
                page: SkillPage {
                    limit: 20,
                    offset: 0
                },
                created_by: Some(scopes()),
            }]
        );
    });
}

#[test]
fn list_keeps_an_explicit_page() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let store = store(repository.clone());
        store
            .list_skills(
                SkillPage {
                    limit: 5,
                    offset: 10,
                },
                Some(&principal("alice")),
            )
            .await
            .unwrap();
        let Effect::FindMany { page, .. } = &page_effects(&repository)[0] else {
            panic!("list must query once");
        };
        assert_eq!(
            *page,
            SkillPage {
                limit: 5,
                offset: 10
            }
        );
    });
}

#[test]
fn list_does_not_filter_an_admin() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let store = store(repository.clone());
        store
            .list_skills(SkillPage::default(), Some(&admin("root")))
            .await
            .unwrap();
        let Effect::FindMany { created_by, .. } = &page_effects(&repository)[0] else {
            panic!("list must query once");
        };
        assert!(created_by.is_none());
    });
}

#[test]
fn list_does_not_filter_a_caller_without_a_principal() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let store = store(repository.clone());
        store.list_skills(SkillPage::default(), None).await.unwrap();
        let Effect::FindMany { created_by, .. } = &page_effects(&repository)[0] else {
            panic!("list must query once");
        };
        assert!(created_by.is_none());
    });
}

#[test]
fn list_returns_no_rows_for_an_identityless_caller_without_a_query() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        repository.update(|state| state.page = vec![stored_row("alice")]);
        let store = store(repository.clone());
        let rows = store
            .list_skills(SkillPage::default(), Some(&anonymous()))
            .await
            .unwrap();
        assert!(rows.is_empty());
        assert!(repository.effects().is_empty());
    });
}

#[test]
fn list_fails_without_a_database() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        repository.update(|state| state.available = false);
        let store = store(repository.clone());
        let error = store
            .list_skills(SkillPage::default(), None)
            .await
            .unwrap_err();
        assert!(matches!(error, StoreError::NoDatabase));
    });
}

#[test]
fn search_lists_the_whole_accessible_set() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let store = store(repository.clone());
        store
            .list_skills_for_search(Some(&principal("alice")))
            .await
            .unwrap();
        assert_eq!(
            page_effects(&repository),
            vec![Effect::FindMany {
                page: SkillPage {
                    limit: 5000,
                    offset: 0
                },
                created_by: Some(scopes()),
            }]
        );
    });
}

#[test]
fn get_reads_once_then_serves_the_cache() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::with_row(stored_row("alice"));
        let store = store(repository.clone());
        let first = store
            .get_skill(SKILL_ID, Some(&principal("alice")))
            .await
            .unwrap();
        let second = store
            .get_skill(SKILL_ID, Some(&principal("alice")))
            .await
            .unwrap();
        assert_eq!(first, stored_row("alice"));
        assert_eq!(second, stored_row("alice"));
        assert_eq!(
            page_effects(&repository),
            vec![Effect::FindUnique(SKILL_ID.to_owned())]
        );
    });
}

#[test]
fn get_denies_a_foreign_tenant_after_the_cache_lookup() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::with_row(stored_row("alice"));
        let store = store(repository.clone());
        store
            .get_skill(SKILL_ID, Some(&principal("alice")))
            .await
            .unwrap();
        let error = store
            .get_skill(SKILL_ID, Some(&principal("bob")))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), format!("Skill not found: {SKILL_ID}"));
        assert_eq!(
            page_effects(&repository),
            vec![Effect::FindUnique(SKILL_ID.to_owned())]
        );
    });
}

#[test]
fn get_allows_an_admin_to_read_any_row() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::with_row(stored_row("alice"));
        let store = store(repository.clone());
        let row = store
            .get_skill(SKILL_ID, Some(&admin("root")))
            .await
            .unwrap();
        assert_eq!(row, stored_row("alice"));
    });
}

#[test]
fn get_allows_a_caller_without_a_principal() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::with_row(stored_row("alice"));
        let store = store(repository.clone());
        assert!(store.get_skill(SKILL_ID, None).await.is_ok());
    });
}

#[test]
fn get_denies_a_row_without_an_owner() {
    support::runtime().block_on(async {
        let mut row = stored_row("alice");
        row.created_by = None;
        let repository = ScriptedRepository::with_row(row);
        let store = store(repository.clone());
        let error = store
            .get_skill(SKILL_ID, Some(&principal("alice")))
            .await
            .unwrap_err();
        assert!(matches!(error, StoreError::NotFound { .. }));
    });
}

#[test]
fn get_caches_a_missing_skill() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let store = store(repository.clone());
        let first = store
            .get_skill(SKILL_ID, Some(&principal("alice")))
            .await
            .unwrap_err();
        let second = store
            .get_skill(SKILL_ID, Some(&principal("alice")))
            .await
            .unwrap_err();
        assert_eq!(first.to_string(), second.to_string());
        assert_eq!(
            page_effects(&repository),
            vec![Effect::FindUnique(SKILL_ID.to_owned())]
        );
    });
}

#[test]
fn get_reports_the_negative_sentinel_text() {
    assert_eq!(NEGATIVE_SKILL_SENTINEL, "__litellm_skill_not_found__");
}

#[test]
fn delete_installs_the_sentinel_only_after_the_row_is_gone() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::with_row(stored_row("alice"));
        let store = store(repository.clone());
        let deleted = store
            .delete_skill(SKILL_ID, Some(&principal("alice")))
            .await
            .unwrap();
        assert_eq!(deleted.id, SKILL_ID);
        assert_eq!(deleted.r#type, "skill_deleted");
        let error = store
            .get_skill(SKILL_ID, Some(&principal("alice")))
            .await
            .unwrap_err();
        assert!(matches!(error, StoreError::NotFound { .. }));
        assert_eq!(
            page_effects(&repository),
            vec![
                Effect::FindUnique(SKILL_ID.to_owned()),
                Effect::Delete(SKILL_ID.to_owned()),
            ]
        );
    });
}

#[test]
fn delete_failure_keeps_the_previous_cache_state() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::with_row(stored_row("alice"));
        repository.update(|state| state.delete_failure = Some("scripted delete failure"));
        let store = store(repository.clone());
        store
            .get_skill(SKILL_ID, Some(&principal("alice")))
            .await
            .unwrap();
        let error = store
            .delete_skill(SKILL_ID, Some(&principal("alice")))
            .await
            .unwrap_err();
        assert!(matches!(error, StoreError::Repository { .. }));
        let row = store
            .get_skill(SKILL_ID, Some(&principal("alice")))
            .await
            .unwrap();
        assert_eq!(row, stored_row("alice"));
        assert_eq!(
            page_effects(&repository),
            vec![
                Effect::FindUnique(SKILL_ID.to_owned()),
                Effect::Delete(SKILL_ID.to_owned()),
            ]
        );
    });
}

#[test]
fn get_serves_a_warm_cache_without_a_database() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::with_row(stored_row("alice"));
        let store = store(repository.clone());
        store
            .get_skill(SKILL_ID, Some(&principal("alice")))
            .await
            .unwrap();
        repository.update(|state| state.available = false);
        let row = store
            .get_skill(SKILL_ID, Some(&principal("alice")))
            .await
            .unwrap();
        assert_eq!(row, stored_row("alice"));
        assert_eq!(
            page_effects(&repository),
            vec![Effect::FindUnique(SKILL_ID.to_owned())]
        );
    });
}

#[test]
fn delete_checks_the_database_before_the_cache() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::with_row(stored_row("alice"));
        let store = store(repository.clone());
        store
            .get_skill(SKILL_ID, Some(&principal("alice")))
            .await
            .unwrap();
        repository.update(|state| state.available = false);
        let error = store
            .delete_skill(SKILL_ID, Some(&principal("alice")))
            .await
            .unwrap_err();
        assert!(matches!(error, StoreError::NoDatabase));
        assert_eq!(
            page_effects(&repository),
            vec![Effect::FindUnique(SKILL_ID.to_owned())]
        );
    });
}

#[test]
fn delete_keeps_the_shared_message_for_missing_rows() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let store = store(repository.clone());
        let error = store
            .delete_skill(SKILL_ID, Some(&principal("alice")))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), format!("Skill not found: {SKILL_ID}"));
    });
}

#[test]
fn delete_denies_a_foreign_tenant() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::with_row(stored_row("alice"));
        let store = store(repository.clone());
        let error = store
            .delete_skill(SKILL_ID, Some(&principal("bob")))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), format!("Skill not found: {SKILL_ID}"));
        assert_eq!(
            page_effects(&repository),
            vec![Effect::FindUnique(SKILL_ID.to_owned())]
        );
        assert!(
            store
                .get_skill(SKILL_ID, Some(&principal("alice")))
                .await
                .is_ok()
        );
    });
}

#[test]
fn prisma_availability_reports_a_missing_client() {
    let repository = PrismaSkillRepository::new(None, support::no_publish());
    assert!(matches!(
        repository.availability(),
        Err(TableError::Database(DbError::NoDbConnected))
    ));
}

#[test]
fn create_does_not_touch_the_cache() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let store = store(repository.clone());
        store
            .create_skill(&new_skill(), Some("alice"), Some(&principal("alice")))
            .await
            .unwrap();
        store
            .get_skill(SKILL_ID, Some(&principal("alice")))
            .await
            .unwrap();
        assert_eq!(
            page_effects(&repository),
            vec![
                Effect::Create(SkillWrite {
                    skill_id: SKILL_ID.to_owned(),
                    display_title: Some("Title".to_owned()),
                    description: None,
                    instructions: Some("Do".to_owned()),
                    source: "custom".to_owned(),
                    metadata: None,
                    file_content: None,
                    file_name: None,
                    file_type: None,
                    created_by: "alice".to_owned(),
                    updated_by: "alice".to_owned(),
                }),
                Effect::FindUnique(SKILL_ID.to_owned()),
            ]
        );
    });
}

#[test]
fn fetch_returns_a_stored_row_silently() {
    let (row, warnings) = support::capture_warnings(|| {
        support::runtime().block_on(async {
            let repository = ScriptedRepository::with_row(stored_row("alice"));
            let store = store(repository);
            store
                .fetch_skill_from_db(SKILL_ID, Some(&principal("alice")))
                .await
        })
    });
    assert_eq!(row, Some(stored_row("alice")));
    assert!(warnings.is_empty());
}

#[test]
fn fetch_silences_a_missing_skill() {
    let (row, warnings) = support::capture_warnings(|| {
        support::runtime().block_on(async {
            let repository = ScriptedRepository::default();
            let store = store(repository);
            store
                .fetch_skill_from_db(SKILL_ID, Some(&principal("alice")))
                .await
        })
    });
    assert_eq!(row, None);
    assert!(warnings.is_empty());
}

#[test]
fn fetch_silences_a_foreign_tenant() {
    let (row, warnings) = support::capture_warnings(|| {
        support::runtime().block_on(async {
            let repository = ScriptedRepository::with_row(stored_row("alice"));
            let store = store(repository);
            store
                .fetch_skill_from_db(SKILL_ID, Some(&principal("bob")))
                .await
        })
    });
    assert_eq!(row, None);
    assert!(warnings.is_empty());
}

#[test]
fn fetch_warns_on_a_repository_failure() {
    let (row, warnings) = support::capture_warnings(|| {
        support::runtime().block_on(async {
            let repository = ScriptedRepository::default();
            repository.update(|state| state.find_failure = Some("scripted find failure"));
            let store = store(repository);
            store
                .fetch_skill_from_db(SKILL_ID, Some(&principal("alice")))
                .await
        })
    });
    assert_eq!(row, None);
    assert_eq!(warnings, vec!["warn".to_owned()]);
}

#[test]
fn fetch_returns_none_without_a_database_and_without_a_warning() {
    let (row, warnings) = support::capture_warnings(|| {
        support::runtime().block_on(async {
            let repository = ScriptedRepository::default();
            repository.update(|state| state.available = false);
            let store = store(repository);
            store
                .fetch_skill_from_db(SKILL_ID, Some(&principal("alice")))
                .await
        })
    });
    assert_eq!(row, None);
    assert!(warnings.is_empty());
}

#[test]
fn availability_is_reported_by_the_repository() {
    let repository = ScriptedRepository::default();
    assert!(repository.availability().is_ok());
    repository.update(|state| state.available = false);
    assert!(repository.availability().is_err());
}
