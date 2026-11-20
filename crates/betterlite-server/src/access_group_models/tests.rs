#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::postgres_store::errors::DbError;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;

use crate::access_group_sync::fixtures::ScriptedCache;
use crate::model_visibility::testing::{Row, Snapshot};

use super::fixtures::{GrantCall, ScriptedFailure, ScriptedModelGrants};
use super::postgres::PostgresModelGrants;
use super::{
    DeploymentCountRow, ModelNameRewrite, ModelRename, ModelSyncError, config_sourced_sibling,
    served_by_a_config_deployment, still_backed, sync_groups_for_deleted_model,
    sync_groups_for_renamed_model,
};

mod postgres;

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn groups(entries: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
    entries
        .iter()
        .map(|(access_group_id, model_names)| ((*access_group_id).to_owned(), names(model_names)))
        .collect()
}

fn assert_groups(database: &ScriptedModelGrants, entries: &[(&str, &[&str])]) {
    assert_eq!(*database.groups.lock().expect("groups"), groups(entries));
}

fn assert_rewrite(database: &ScriptedModelGrants, expected: ModelNameRewrite) {
    let calls = database.calls.lock().expect("calls");
    assert_eq!(calls.get(1), Some(&GrantCall::Rewrite(expected)));
}

fn counting_database(entries: &[(&str, &[&str])], counts: &[i32]) -> ScriptedModelGrants {
    let database = ScriptedModelGrants::new(groups(entries));
    *database.deployment_counts.lock().expect("counts") = counts
        .iter()
        .map(|count| DeploymentCountRow {
            deployment_count: *count,
        })
        .collect();
    database
}

fn backed_database(entries: &[(&str, &[&str])]) -> ScriptedModelGrants {
    counting_database(entries, &[1])
}

fn info(id: &str, db_model: Option<bool>) -> JsonObject {
    let mut info = JsonObject::new();
    info.insert("id".to_owned(), Json::Text(id.to_owned()));
    if let Some(flag) = db_model {
        info.insert("db_model".to_owned(), Json::Bool(flag));
    }
    info
}

// Router(model_list=...): the rows as the config-sibling lookups read them.
fn catalog(rows: &[(&str, JsonObject)]) -> DeploymentCatalog {
    let params = JsonObject::new();
    let infos: Vec<Json> = rows
        .iter()
        .map(|(_, info)| Json::Object(info.clone()))
        .collect();
    let snapshot_rows: Vec<Row<'_>> = rows
        .iter()
        .zip(&infos)
        .map(|((model_name, _), model_info)| Row {
            model_name,
            litellm_params: &params,
            model_info,
        })
        .collect();
    Snapshot::build(&snapshot_rows, &JsonObject::new())
        .expect("the catalog snapshot")
        .catalog
}

#[test]
fn own_id_not_sibling() {
    let catalog = catalog(&[("gpt-4o", info("dep-1", Some(false)))]);
    assert!(!config_sourced_sibling(&catalog, "dep-1", "dep-1").unwrap());
}

#[test]
fn absent_or_db_sibling() {
    let catalog = catalog(&[
        ("gpt-4o", info("db-1", Some(true))),
        ("gpt-4o", info("config-1", None)),
        ("gpt-4o", info("config-2", Some(false))),
    ]);
    assert!(!config_sourced_sibling(&catalog, "db-1", "current").unwrap());
    assert!(!config_sourced_sibling(&catalog, "missing", "current").unwrap());
    assert!(config_sourced_sibling(&catalog, "config-1", "current").unwrap());
    assert!(config_sourced_sibling(&catalog, "config-2", "current").unwrap());
}

#[tokio::test]
async fn config_sibling_short_circuit() {
    let catalog = catalog(&[
        ("gpt-4o", info("db-1", Some(true))),
        ("gpt-4o", info("config-1", None)),
    ]);
    let mut database = ScriptedModelGrants::new(groups(&[]));
    database.failure = Some(ScriptedFailure::Count);
    let cache = ScriptedCache::new();
    assert!(served_by_a_config_deployment(Some(&catalog), "gpt-4o", "current").unwrap());
    assert!(
        still_backed(&database, Some(&catalog), "gpt-4o", "current")
            .await
            .unwrap()
    );
    let rename = ModelRename {
        model_id: "current",
        old_name: "gpt-4o",
        new_name: "gpt-4.1",
    };
    sync_groups_for_renamed_model(&database, &cache, Some(&catalog), &rename)
        .await
        .unwrap();
    let calls = database.calls.lock().expect("calls");
    assert_eq!(calls.len(), 1);
    assert!(matches!(
        calls.first(),
        Some(GrantCall::Rewrite(ModelNameRewrite::Append { .. }))
    ));
    drop(calls);
    assert!(cache.calls.lock().expect("calls").is_empty());
}

#[tokio::test]
async fn database_count_includes_current_id() {
    let single = counting_database(&[], &[1_i32]);
    assert!(still_backed(&single, None, "solo", "dep-1").await.unwrap());
    let any_positive = counting_database(&[], &[0_i32, 1_i32]);
    assert!(
        still_backed(&any_positive, None, "solo", "dep-1")
            .await
            .unwrap()
    );
    let none_positive = counting_database(&[], &[0_i32, 0_i32]);
    assert!(
        !still_backed(&none_positive, None, "solo", "dep-1")
            .await
            .unwrap()
    );
    let calls = single.calls.lock().expect("calls");
    assert_eq!(
        *calls,
        vec![GrantCall::Count {
            model_name: "solo".to_owned()
        }],
        "the count query is bound to the name alone, so the current deployment counts too"
    );
}

#[tokio::test]
async fn rename_same_no_io() {
    let database = PostgresModelGrants { client: None };
    let cache = ScriptedCache::new();
    let rename = ModelRename {
        model_id: "dep-1",
        old_name: "gpt-4o",
        new_name: "gpt-4o",
    };
    sync_groups_for_renamed_model(&database, &cache, None, &rename)
        .await
        .unwrap();
    assert!(cache.calls.lock().expect("calls").is_empty());
}

#[tokio::test]
async fn rename_backed_appends() {
    let mut database = backed_database(&[
        ("g1", &["old"]),
        ("g2", &["old", "new"]),
        ("g3", &["other"]),
        ("g5", &["old"]),
    ]);
    database.reverse_rows = true;
    let cache = ScriptedCache::new();
    let rename = ModelRename {
        model_id: "dep-1",
        old_name: "old",
        new_name: "new",
    };
    sync_groups_for_renamed_model(&database, &cache, None, &rename)
        .await
        .unwrap();
    assert_eq!(
        *cache.calls.lock().expect("calls"),
        names(&["g5", "g1"]),
        "the returned row order reaches the invalidator"
    );
    assert_groups(
        &database,
        &[
            ("g1", &["old", "new"]),
            ("g2", &["old", "new"]),
            ("g3", &["other"]),
            ("g5", &["old", "new"]),
        ],
    );
    assert_rewrite(
        &database,
        ModelNameRewrite::Append {
            old_name: "old".to_owned(),
            new_name: "new".to_owned(),
        },
    );
}

#[tokio::test]
async fn replace_preserves_old_positions() {
    let database = ScriptedModelGrants::new(groups(&[
        ("g1", &["old", "x", "old"]),
        ("g2", &["new", "old", "new"]),
        ("g3", &["old", "old"]),
        ("g4", &["other"]),
    ]));
    let cache = ScriptedCache::new();
    let rename = ModelRename {
        model_id: "dep-1",
        old_name: "old",
        new_name: "new",
    };
    sync_groups_for_renamed_model(&database, &cache, None, &rename)
        .await
        .unwrap();
    assert_eq!(
        *cache.calls.lock().expect("calls"),
        names(&["g1", "g2", "g3"])
    );
    assert_groups(
        &database,
        &[
            ("g1", &["new", "x", "new"]),
            ("g2", &["new"]),
            ("g3", &["new", "new"]),
            ("g4", &["other"]),
        ],
    );
    assert_rewrite(
        &database,
        ModelNameRewrite::Replace {
            old_name: "old".to_owned(),
            new_name: "new".to_owned(),
        },
    );
}

#[tokio::test]
async fn delete_backed_noop() {
    let database = backed_database(&[("g1", &["old"])]);
    let cache = ScriptedCache::new();
    sync_groups_for_deleted_model(&database, &cache, None, "dep-1", "old")
        .await
        .unwrap();
    assert!(cache.calls.lock().expect("calls").is_empty());
    assert_eq!(
        database.groups.lock().expect("groups").get("g1"),
        Some(&names(&["old"]))
    );
    assert_eq!(
        *database.calls.lock().expect("calls"),
        vec![GrantCall::Count {
            model_name: "old".to_owned()
        }]
    );
}

#[tokio::test]
async fn delete_last_name() {
    let database = ScriptedModelGrants::new(groups(&[
        ("g1", &["old", "x", "old"]),
        ("g2", &["old"]),
        ("g3", &["new"]),
    ]));
    let cache = ScriptedCache::new();
    sync_groups_for_deleted_model(&database, &cache, None, "dep-1", "old")
        .await
        .unwrap();
    assert_eq!(*cache.calls.lock().expect("calls"), names(&["g1", "g2"]));
    assert_groups(&database, &[("g1", &["x"]), ("g2", &[]), ("g3", &["new"])]);
    sync_groups_for_deleted_model(&database, &cache, None, "dep-1", "old")
        .await
        .unwrap();
    assert_eq!(
        *cache.calls.lock().expect("calls"),
        names(&["g1", "g2"]),
        "a retry that touches no rows evicts nothing more"
    );
}

#[tokio::test]
async fn database_failure_stops_before_cache() {
    let mut database = ScriptedModelGrants::new(groups(&[("g1", &["old"])]));
    database.failure = Some(ScriptedFailure::Count);
    let cache = ScriptedCache::new();
    let rename = ModelRename {
        model_id: "dep-1",
        old_name: "old",
        new_name: "new",
    };
    let error = sync_groups_for_renamed_model(&database, &cache, None, &rename)
        .await
        .expect_err("the count query fails");
    assert!(matches!(
        error,
        ModelSyncError::Database(DbError::NoDbConnected)
    ));
    assert!(cache.calls.lock().expect("calls").is_empty());

    let mut failing_rewrite = ScriptedModelGrants::new(groups(&[("g1", &["old"])]));
    failing_rewrite.failure = Some(ScriptedFailure::Rewrite);
    let rewrite_error = sync_groups_for_renamed_model(&failing_rewrite, &cache, None, &rename)
        .await
        .expect_err("the rewrite fails");
    assert!(matches!(
        rewrite_error,
        ModelSyncError::Database(DbError::NoDbConnected)
    ));
    assert!(cache.calls.lock().expect("calls").is_empty());
    assert_eq!(
        failing_rewrite.groups.lock().expect("groups").get("g1"),
        Some(&names(&["old"]))
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
