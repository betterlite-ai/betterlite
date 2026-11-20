#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_db::postgres_store::errors::DbError;

use crate::access_group_models::fixtures::{ScriptedFailure, ScriptedModelGrants};
use crate::access_group_models::{DeploymentCountRow, ModelRename, ModelSyncError};

use super::cache::cache_keys;
use super::fixtures::{
    AllowlistEvent, EventLog, ScriptedAllowlistCache, ScriptedAllowlists, ScriptedRow, row,
};
use super::postgres::PostgresAllowlists;
use super::statement::{AllowlistKind, AllowlistRewrite, TouchedRow};
use super::{ModelAllowlistError, sync_renamed_model_allowlists};

mod adapter;
mod postgres;

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

const fn rename<'request>(
    old_name: &'request str,
    new_name: &'request str,
) -> ModelRename<'request> {
    ModelRename {
        model_id: "dep-1",
        old_name,
        new_name,
    }
}

// still_backed's database count: zero rows means no other deployment serves the old name.
fn grant_counts(count: i32) -> ScriptedModelGrants {
    let database = ScriptedModelGrants::new(BTreeMap::new());
    *database.deployment_counts.lock().expect("counts") = vec![DeploymentCountRow {
        deployment_count: count,
    }];
    database
}

fn arrays(database: &ScriptedAllowlists) -> Vec<Option<Vec<String>>> {
    let stored = database.rows.lock().expect("rows");
    stored.iter().map(|entry| entry.models.clone()).collect()
}

fn calls(cache: &ScriptedAllowlistCache) -> Vec<Vec<String>> {
    let calls = cache.calls.lock().expect("calls");
    calls.clone()
}

fn touched(kind: AllowlistKind, object_id: &str, team_alias: Option<&str>) -> TouchedRow {
    TouchedRow {
        kind,
        object_id: object_id.to_owned(),
        team_alias: team_alias.map(str::to_owned),
    }
}

fn entity_rows() -> Vec<ScriptedRow> {
    vec![
        row(AllowlistKind::Team, "t-1", Some("alias-1"), Some(&["old"])),
        row(AllowlistKind::Key, "key-1", None, Some(&["old"])),
        row(AllowlistKind::Organization, "org-1", None, Some(&["old"])),
        row(AllowlistKind::Project, "project-1", None, Some(&["old"])),
        row(AllowlistKind::User, "user-1", None, Some(&["old"])),
    ]
}

const ENTITY_KEYS: [&str; 7] = [
    "team_id:t-1",
    "team_alias:alias-1",
    "key-1",
    "org_id:org-1",
    "org_id:org-1:with_budget",
    "project_id:project-1",
    "user-1",
];

#[tokio::test]
async fn all_entity_allowlists() {
    let events = EventLog::default();
    let database = ScriptedAllowlists::new(entity_rows(), Arc::clone(&events));
    let cache = ScriptedAllowlistCache::recording(Arc::clone(&events));
    let request = rename("old", "new");
    sync_renamed_model_allowlists(&grant_counts(0), &database, &cache, None, &request)
        .await
        .unwrap();
    assert_eq!(
        *database.calls.lock().expect("calls"),
        vec![AllowlistRewrite::Replace {
            old_name: "old".to_owned(),
            new_name: "new".to_owned(),
        }]
    );
    assert_eq!(arrays(&database), vec![Some(names(&["new"])); 5]);
    assert_eq!(calls(&cache), vec![names(&ENTITY_KEYS)]);
    let recorded = events.lock().expect("events").clone();
    assert_eq!(recorded.first(), Some(&AllowlistEvent::SqlCompleted));
    assert_eq!(recorded.len(), 2);
}

#[tokio::test]
async fn duplicate_target_name() {
    let mut database = ScriptedAllowlists::new(
        vec![
            row(AllowlistKind::Key, "k-1", None, Some(&["old", "x", "old"])),
            row(
                AllowlistKind::Key,
                "k-2",
                None,
                Some(&["new", "old", "new"]),
            ),
            row(AllowlistKind::Key, "k-3", None, Some(&["old", "old"])),
            row(AllowlistKind::Key, "k-4", None, Some(&["other"])),
        ],
        EventLog::default(),
    );
    // The keys follow the order the statement returned, which SQL does not promise to keep.
    database.reverse_rows = true;
    let cache = ScriptedAllowlistCache::recording(EventLog::default());
    let request = rename("old", "new");
    sync_renamed_model_allowlists(&grant_counts(0), &database, &cache, None, &request)
        .await
        .unwrap();
    assert_eq!(
        arrays(&database),
        vec![
            Some(names(&["new", "x", "new"])),
            Some(names(&["new"])),
            Some(names(&["new", "new"])),
            Some(names(&["other"])),
        ]
    );
    assert_eq!(calls(&cache), vec![names(&["k-3", "k-2", "k-1"])]);
}

#[tokio::test]
async fn exact_name_only() {
    let events = EventLog::default();
    let database = ScriptedAllowlists::new(
        vec![
            row(AllowlistKind::Key, "k-case", None, Some(&["Old"])),
            row(AllowlistKind::Key, "k-wildcard", None, Some(&["gpt-*"])),
            row(AllowlistKind::Key, "k-exact", None, Some(&["old"])),
            row(AllowlistKind::User, "u-case", None, Some(&["OLD"])),
        ],
        Arc::clone(&events),
    );
    let first = ScriptedAllowlistCache::recording(Arc::clone(&events));
    let request = rename("old", "new");
    sync_renamed_model_allowlists(&grant_counts(0), &database, &first, None, &request)
        .await
        .unwrap();
    assert_eq!(calls(&first), vec![names(&["k-exact"])]);
    let second = ScriptedAllowlistCache::recording(events);
    let wildcard = rename("gpt-*", "gpt-4o");
    sync_renamed_model_allowlists(&grant_counts(0), &database, &second, None, &wildcard)
        .await
        .unwrap();
    assert_eq!(calls(&second), vec![names(&["k-wildcard"])]);
    assert_eq!(
        arrays(&database),
        vec![
            Some(names(&["Old"])),
            Some(names(&["gpt-4o"])),
            Some(names(&["new"])),
            Some(names(&["OLD"])),
        ]
    );
}

#[tokio::test]
async fn key_cache_invalidation() {
    let database = ScriptedAllowlists::new(
        vec![
            row(AllowlistKind::Key, "sk-abc", None, Some(&["old"])),
            row(AllowlistKind::Key, "", None, Some(&["old"])),
        ],
        EventLog::default(),
    );
    let cache = ScriptedAllowlistCache::recording(EventLog::default());
    let request = rename("old", "new");
    sync_renamed_model_allowlists(&grant_counts(0), &database, &cache, None, &request)
        .await
        .unwrap();
    assert_eq!(calls(&cache), vec![names(&["sk-abc", ""])]);
}

#[tokio::test]
async fn team_cache_invalidation() {
    let database = ScriptedAllowlists::new(
        vec![
            row(AllowlistKind::Team, "t-1", Some("prod"), Some(&["old"])),
            row(AllowlistKind::Team, "t-2", Some(""), Some(&["old"])),
            row(AllowlistKind::Team, "t-3", None, Some(&["old"])),
            row(AllowlistKind::Team, "t-4", Some("prod"), Some(&["old"])),
        ],
        EventLog::default(),
    );
    let cache = ScriptedAllowlistCache::recording(EventLog::default());
    let request = rename("old", "new");
    sync_renamed_model_allowlists(&grant_counts(0), &database, &cache, None, &request)
        .await
        .unwrap();
    assert_eq!(
        calls(&cache),
        vec![names(&[
            "team_id:t-1",
            "team_alias:prod",
            "team_id:t-2",
            "team_id:t-3",
            "team_id:t-4",
            "team_alias:prod",
        ])]
    );
}

#[tokio::test]
async fn rename_same_no_io() {
    let mut model_grants = ScriptedModelGrants::new(BTreeMap::new());
    model_grants.failure = Some(ScriptedFailure::Count);
    let events = EventLog::default();
    let cache = ScriptedAllowlistCache::recording(Arc::clone(&events));
    let request = rename("same", "same");
    sync_renamed_model_allowlists(
        &model_grants,
        &PostgresAllowlists { client: None },
        &cache,
        None,
        &request,
    )
    .await
    .unwrap();
    assert!(calls(&cache).is_empty());
    assert!(events.lock().expect("events").is_empty());
}

#[tokio::test]
async fn append_when_backed() {
    let database = ScriptedAllowlists::new(
        vec![
            row(AllowlistKind::Team, "t-1", Some("alias-1"), Some(&["old"])),
            row(AllowlistKind::Key, "k-both", None, Some(&["old", "new"])),
            row(AllowlistKind::User, "u-other", None, Some(&["other"])),
        ],
        EventLog::default(),
    );
    let cache = ScriptedAllowlistCache::recording(EventLog::default());
    let request = rename("old", "new");
    sync_renamed_model_allowlists(&grant_counts(1), &database, &cache, None, &request)
        .await
        .unwrap();
    assert_eq!(
        *database.calls.lock().expect("calls"),
        vec![AllowlistRewrite::Append {
            old_name: "old".to_owned(),
            new_name: "new".to_owned(),
        }]
    );
    assert_eq!(
        arrays(&database),
        vec![
            Some(names(&["old", "new"])),
            Some(names(&["old", "new"])),
            Some(names(&["other"])),
        ]
    );
    assert_eq!(
        calls(&cache),
        vec![names(&["team_id:t-1", "team_alias:alias-1"])]
    );
}

#[tokio::test]
async fn sibling_query_failure_stops_before_statement() {
    let mut model_grants = ScriptedModelGrants::new(BTreeMap::new());
    model_grants.failure = Some(ScriptedFailure::Count);
    let events = EventLog::default();
    let database = ScriptedAllowlists::new(
        vec![row(AllowlistKind::Key, "k-1", None, Some(&["old"]))],
        Arc::clone(&events),
    );
    let cache = ScriptedAllowlistCache::recording(events);
    let request = rename("old", "new");
    let error = sync_renamed_model_allowlists(&model_grants, &database, &cache, None, &request)
        .await
        .expect_err("the sibling query fails");
    assert!(matches!(
        error,
        ModelAllowlistError::Grants(ModelSyncError::Database(DbError::NoDbConnected))
    ));
    assert!(database.calls.lock().expect("calls").is_empty());
    assert!(calls(&cache).is_empty());
}

#[tokio::test]
async fn statement_failure_stops_before_cache() {
    let mut database = ScriptedAllowlists::new(
        vec![row(AllowlistKind::Key, "k-1", None, Some(&["old"]))],
        EventLog::default(),
    );
    database.failure = true;
    let events = EventLog::default();
    let cache = ScriptedAllowlistCache::recording(Arc::clone(&events));
    let request = rename("old", "new");
    let error = sync_renamed_model_allowlists(&grant_counts(0), &database, &cache, None, &request)
        .await
        .expect_err("the statement fails");
    assert!(matches!(
        error,
        ModelAllowlistError::Database(DbError::NoDbConnected)
    ));
    assert!(calls(&cache).is_empty());
    assert!(events.lock().expect("events").is_empty());
    assert_eq!(arrays(&database), vec![Some(names(&["old"]))]);
}

#[tokio::test]
async fn empty_rewrite_evicts_empty_tuple() {
    let events = EventLog::default();
    let database = ScriptedAllowlists::new(Vec::new(), Arc::clone(&events));
    let cache = ScriptedAllowlistCache::recording(Arc::clone(&events));
    let request = rename("absent", "new");
    sync_renamed_model_allowlists(&grant_counts(0), &database, &cache, None, &request)
        .await
        .unwrap();
    assert_eq!(calls(&cache), vec![Vec::<String>::new()]);
    assert_eq!(
        events.lock().expect("events").clone(),
        vec![
            AllowlistEvent::SqlCompleted,
            AllowlistEvent::CacheCall(Vec::new()),
        ]
    );
}

#[tokio::test]
async fn cache_failure_follows_committed_statement() {
    let events = EventLog::default();
    let database = ScriptedAllowlists::new(
        vec![row(AllowlistKind::Team, "t-1", None, Some(&["old"]))],
        Arc::clone(&events),
    );
    let cache = ScriptedAllowlistCache::failing(Arc::clone(&events), 3);
    let request = rename("old", "new");
    let error = sync_renamed_model_allowlists(&grant_counts(0), &database, &cache, None, &request)
        .await
        .expect_err("the eviction reports an error");
    assert!(matches!(error, ModelAllowlistError::Cache(_)));
    assert_eq!(arrays(&database), vec![Some(names(&["new"]))]);
    assert_eq!(calls(&cache), vec![names(&["team_id:t-1"])]);
    assert_eq!(
        events.lock().expect("events").clone(),
        vec![
            AllowlistEvent::SqlCompleted,
            AllowlistEvent::CacheCall(names(&["team_id:t-1"])),
        ],
        "the statement completes before the one cache call, and the error is not retried"
    );
}

const EXPECTED_TABLES: [(&str, &str, &str, &str); 5] = [
    ("team", "LiteLLM_TeamTable", "team_id", "\"team_alias\""),
    ("key", "LiteLLM_VerificationToken", "token", "NULL::text"),
    (
        "org",
        "LiteLLM_OrganizationTable",
        "organization_id",
        "NULL::text",
    ),
    (
        "project",
        "LiteLLM_ProjectTable",
        "project_id",
        "NULL::text",
    ),
    ("user", "LiteLLM_UserTable", "user_id", "NULL::text"),
];

fn expected_sql(set_clause: &str, where_clause: &str) -> String {
    let ctes: Vec<String> = EXPECTED_TABLES
        .iter()
        .map(|(kind, table, id_column, alias)| {
            format!(
                "{kind}_rows AS (UPDATE \"{table}\" SET \"models\" = {set_clause} WHERE {where_clause} RETURNING '{kind}' AS kind, \"{id_column}\" AS object_id, {alias} AS team_alias)"
            )
        })
        .collect();
    let rows: Vec<String> = EXPECTED_TABLES
        .iter()
        .map(|(kind, _, _, _)| format!("SELECT kind, object_id, team_alias FROM {kind}_rows"))
        .collect();
    let joined_ctes = ctes.join(", ");
    let joined_rows = rows.join(" UNION ALL ");
    format!("WITH {joined_ctes} {joined_rows}")
}

#[test]
fn rewrite_statement_text() {
    let replace = AllowlistRewrite::Replace {
        old_name: "old".to_owned(),
        new_name: "new".to_owned(),
    };
    assert_eq!(
        replace.sql(),
        expected_sql(
            "array_replace(array_remove(\"models\", $2), $1, $2)",
            "$1 = ANY(\"models\")"
        )
    );
    let append = AllowlistRewrite::Append {
        old_name: "old".to_owned(),
        new_name: "new".to_owned(),
    };
    assert_eq!(
        append.sql(),
        expected_sql(
            "array_append(\"models\", $2)",
            "$1 = ANY(\"models\") AND NOT ($2 = ANY(\"models\"))"
        )
    );
}

#[test]
fn unknown_kind_rejected() {
    let error = TouchedRow::from_values("ghost", "object-1".to_owned(), None)
        .expect_err("an unknown kind is refused");
    assert!(matches!(error, ModelAllowlistError::UnknownKind { kind } if kind == "ghost"));
    let parsed = TouchedRow::from_values("org", "object-1".to_owned(), Some("alias".to_owned()))
        .expect("the org kind");
    assert_eq!(parsed.kind, AllowlistKind::Organization);
    assert_eq!(parsed.object_id, "object-1");
    assert_eq!(parsed.team_alias.as_deref(), Some("alias"));
}

#[test]
fn cache_keys_per_kind() {
    assert_eq!(
        cache_keys(&touched(AllowlistKind::Team, "t-1", Some("alias-1"))),
        names(&["team_id:t-1", "team_alias:alias-1"])
    );
    assert_eq!(
        cache_keys(&touched(AllowlistKind::Team, "t-1", Some(""))),
        names(&["team_id:t-1"])
    );
    assert_eq!(
        cache_keys(&touched(AllowlistKind::Team, "t-1", None)),
        names(&["team_id:t-1"])
    );
    assert_eq!(
        cache_keys(&touched(AllowlistKind::Team, "", None)),
        names(&["team_id:"])
    );
    assert_eq!(
        cache_keys(&touched(AllowlistKind::Key, "", None)),
        names(&[""])
    );
    assert_eq!(
        cache_keys(&touched(AllowlistKind::Organization, "", None)),
        names(&["org_id:", "org_id::with_budget"])
    );
    assert_eq!(
        cache_keys(&touched(AllowlistKind::Project, "", None)),
        names(&["project_id:"])
    );
    assert_eq!(
        cache_keys(&touched(AllowlistKind::User, "", None)),
        names(&[""])
    );
}

#[tokio::test]
async fn postgres_pinned_rows() {
    let (fixture, pool) = postgres::database().await;
    postgres::pinned_rows(&pool).await;
    postgres::close(fixture, pool).await;
}

#[tokio::test]
async fn postgres_failure_effects() {
    let (fixture, pool) = postgres::database().await;
    postgres::failure_effects(&pool).await;
    postgres::close(fixture, pool).await;
}
