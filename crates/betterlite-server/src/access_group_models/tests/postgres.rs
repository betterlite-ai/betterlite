use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use futures_util::future::join_all;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use crate::access_group_models::postgres::PostgresModelGrants;
use crate::access_group_models::{
    DeploymentCountRow, ModelGrantDatabase as _, ModelNameRewrite, ModelRename, ModelSyncError,
    TouchedGroupRow, still_backed, sync_groups_for_deleted_model, sync_groups_for_renamed_model,
};
use crate::access_group_sync::fixtures::ScriptedCache;

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_ProxyModelTable" (
    "model_id" TEXT PRIMARY KEY,
    "model_name" TEXT NOT NULL
);
CREATE TABLE "LiteLLM_AccessGroupTable" (
    "access_group_id" TEXT PRIMARY KEY,
    "access_model_names" TEXT[]
);
"#;
const INSERT_MODEL_SQL: &str =
    r#"INSERT INTO "LiteLLM_ProxyModelTable" ("model_id", "model_name") VALUES ($1, $2)"#;
const INSERT_GROUP_SQL: &str = r#"INSERT INTO "LiteLLM_AccessGroupTable" ("access_group_id", "access_model_names") VALUES ($1, $2)"#;
const MODEL_NAMES_SQL: &str =
    r#"SELECT "access_model_names" FROM "LiteLLM_AccessGroupTable" WHERE "access_group_id" = $1"#;
const NULL_ID_NAMES_SQL: &str = r#"SELECT "access_model_names" FROM "LiteLLM_AccessGroupTable" WHERE "access_group_id" IS NULL"#;
const MODELS: [(&str, &str); 4] = [
    ("dep-1", "solo"),
    ("dep-2", "shared"),
    ("dep-3", "shared"),
    ("dep-9", "kept"),
];
const GROUPS: [(&str, Option<&[&str]>); 15] = [
    ("g1", Some(&["old", "old"])),
    ("g2", Some(&["new", "old", "new"])),
    ("g3", None),
    ("g4", Some(&[])),
    ("g5", Some(&["other"])),
    ("g6", Some(&["it's $1"])),
    ("g7", Some(&["old", "new"])),
    ("g9", Some(&["old2", "x", "old2"])),
    ("g10", Some(&["new2", "old2", "new2"])),
    ("g11", Some(&["old2", "old2"])),
    ("g12", Some(&["old3", "x", "old3"])),
    ("g13", Some(&["old3"])),
    ("g14", Some(&["old4"])),
    ("g15", Some(&["kept"])),
    ("g16", Some(&["gone"])),
];

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(super) async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["access-group-models-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
        .resolve()
        .await
        .expect("the database fixture");
    let manager = Manager::new(
        fixture
            .options
            .url
            .expose_secret()
            .parse()
            .expect("the database url"),
        NoTls,
    );
    let pool = Pool::builder(manager)
        .max_size(4)
        .build()
        .expect("the database pool");
    (fixture, pool)
}

pub(super) async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.expect("the fixture cleanup");
}

async fn execute(pool: &Pool, statement: &str, parameters: &[&(dyn ToSql + Sync)]) {
    let client = pool.get().await.expect("a fixture connection");
    client
        .execute(statement, parameters)
        .await
        .expect("the fixture statement");
}

async fn insert_model(pool: &Pool, model_id: &str, model_name: &str) {
    let parameters: [&(dyn ToSql + Sync); 2] = [&model_id, &model_name];
    execute(pool, INSERT_MODEL_SQL, &parameters).await;
}

async fn insert_group(pool: &Pool, access_group_id: Option<&str>, model_names: Option<&[String]>) {
    let parameters: [&(dyn ToSql + Sync); 2] = [&access_group_id, &model_names];
    execute(pool, INSERT_GROUP_SQL, &parameters).await;
}

fn expected(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

async fn model_names(pool: &Pool, access_group_id: &str) -> Option<Vec<String>> {
    let client = pool.get().await.expect("a fixture connection");
    let row = client
        .query_one(MODEL_NAMES_SQL, &[&access_group_id])
        .await
        .expect("the access group row");
    row.get(0)
}

fn touched_ids(rows: &[TouchedGroupRow]) -> Vec<String> {
    let mut ids: Vec<String> = rows.iter().map(|row| row.access_group_id.clone()).collect();
    ids.sort();
    ids
}

const fn grants<'db>(
    writer: &'db PostgresStore,
    reader: &'db PostgresStore,
    availability: &'db Availability,
) -> PostgresModelGrants<'db> {
    PostgresModelGrants {
        client: Some(PrismaClient {
            writer,
            reader,
            availability,
        }),
    }
}

async fn seed(pool: &Pool) {
    for (model_id, model_name) in MODELS {
        insert_model(pool, model_id, model_name).await;
    }
    for (access_group_id, names) in GROUPS {
        let stored_names = names.map(expected);
        insert_group(pool, Some(access_group_id), stored_names.as_deref()).await;
    }
}

pub(super) async fn pinned_rows(pool: &Pool) {
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let availability = Availability::default();
    let grants = grants(&store, &store, &availability);
    seed(pool).await;
    count_rows(&grants).await;
    apply_rewrites(pool, &grants).await;
    availability
        .writer_unavailable
        .store(true, Ordering::SeqCst);
    degraded_rows(pool, &grants).await;
}

async fn count_rows(grants: &PostgresModelGrants<'_>) {
    assert_eq!(
        grants.backing_deployments("solo").await.unwrap(),
        vec![DeploymentCountRow {
            deployment_count: 1
        }]
    );
    assert_eq!(
        grants.backing_deployments("shared").await.unwrap(),
        vec![DeploymentCountRow {
            deployment_count: 2
        }]
    );
    assert_eq!(
        grants.backing_deployments("absent").await.unwrap(),
        vec![DeploymentCountRow {
            deployment_count: 0
        }]
    );
    assert!(still_backed(grants, None, "solo", "dep-1").await.unwrap());
    assert!(!still_backed(grants, None, "absent", "dep-1").await.unwrap());
}

async fn apply_rewrites(pool: &Pool, grants: &PostgresModelGrants<'_>) {
    let rewrites = [
        ModelNameRewrite::Append {
            old_name: "old".to_owned(),
            new_name: "new".to_owned(),
        },
        ModelNameRewrite::Remove {
            model_name: "old3".to_owned(),
        },
        ModelNameRewrite::Replace {
            old_name: "old2".to_owned(),
            new_name: "new2".to_owned(),
        },
        ModelNameRewrite::Remove {
            model_name: "it's $1".to_owned(),
        },
    ];
    let mut touched = Vec::new();
    for rewrite in &rewrites {
        let rows = grants.rewrite_model_names(rewrite).await.unwrap();
        touched.push(touched_ids(&rows));
    }
    assert_eq!(
        touched,
        vec![
            expected(&["g1"]),
            expected(&["g12", "g13"]),
            expected(&["g10", "g11", "g9"]),
            expected(&["g6"]),
        ]
    );
    assert_stored_rows(pool).await;
}

async fn assert_stored_rows(pool: &Pool) {
    let ids = [
        "g1", "g2", "g3", "g4", "g7", "g12", "g13", "g9", "g10", "g11", "g6",
    ];
    let actual = join_all(ids.iter().map(|id| model_names(pool, id))).await;
    assert_eq!(
        actual,
        [
            Some(expected(&["old", "old", "new"])),
            Some(expected(&["new", "old", "new"])),
            None,
            Some(Vec::new()),
            Some(expected(&["old", "new"])),
            Some(expected(&["x", "x"])),
            Some(Vec::new()),
            Some(expected(&["new2", "x", "new2"])),
            Some(expected(&["new2"])),
            Some(expected(&["new2", "new2"])),
            Some(Vec::new()),
        ]
    );
}

async fn degraded_rows(pool: &Pool, grants: &PostgresModelGrants<'_>) {
    let removed = grants
        .rewrite_model_names(&ModelNameRewrite::Remove {
            model_name: "old4".to_owned(),
        })
        .await
        .unwrap();
    assert_eq!(touched_ids(&removed), expected(&["g14"]));
    assert_eq!(model_names(pool, "g14").await, Some(Vec::new()));
}

fn failing_cache(access_group_id: &str) -> ScriptedCache {
    let mut cache = ScriptedCache::new();
    cache.failures = Mutex::new(BTreeMap::from([(access_group_id.to_owned(), 3)]));
    cache
}

pub(super) async fn failure_effects(pool: &Pool) {
    seed(pool).await;
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let availability = Availability::default();
    let grants = grants(&store, &store, &availability);
    replace_cache_failure(pool, &grants).await;
    other_cache_failures(pool, &grants).await;
    invalid_row_after_commit(pool, &grants).await;
}

async fn replace_cache_failure(pool: &Pool, grants: &PostgresModelGrants<'_>) {
    let replace = ModelRename {
        model_id: "dep-1",
        old_name: "old",
        new_name: "new",
    };
    let cache = failing_cache("g1");
    let replace_error = sync_groups_for_renamed_model(grants, &cache, None, &replace)
        .await
        .expect_err("the eviction fails after the rewrite committed");
    assert!(matches!(replace_error, ModelSyncError::Cache(_)));
    assert_eq!(
        model_names(pool, "g1").await,
        Some(expected(&["new", "new"]))
    );
    assert_eq!(model_names(pool, "g2").await, Some(expected(&["new"])));
    assert_eq!(model_names(pool, "g7").await, Some(expected(&["new"])));
    let mut attempted = cache.calls.lock().expect("calls").clone();
    attempted.sort();
    assert_eq!(attempted, expected(&["g1", "g2", "g7"]));
    let replace_retry = ScriptedCache::new();
    sync_groups_for_renamed_model(grants, &replace_retry, None, &replace)
        .await
        .unwrap();
    assert!(
        replace_retry.calls.lock().expect("calls").is_empty(),
        "a retry whose replace changes no rows cannot reconstruct the touched set"
    );
}

async fn other_cache_failures(pool: &Pool, grants: &PostgresModelGrants<'_>) {
    let append = ModelRename {
        model_id: "dep-1",
        old_name: "kept",
        new_name: "kept2",
    };
    let append_error = sync_groups_for_renamed_model(grants, &failing_cache("g15"), None, &append)
        .await
        .expect_err("the append eviction fails");
    assert!(matches!(append_error, ModelSyncError::Cache(_)));
    assert_eq!(
        model_names(pool, "g15").await,
        Some(expected(&["kept", "kept2"]))
    );
    let append_retry = ScriptedCache::new();
    sync_groups_for_renamed_model(grants, &append_retry, None, &append)
        .await
        .unwrap();
    assert!(append_retry.calls.lock().expect("calls").is_empty());

    let delete_error =
        sync_groups_for_deleted_model(grants, &failing_cache("g16"), None, "dep-1", "gone")
            .await
            .expect_err("the delete eviction fails");
    assert!(matches!(delete_error, ModelSyncError::Cache(_)));
    assert_eq!(model_names(pool, "g16").await, Some(Vec::new()));
    let delete_retry = ScriptedCache::new();
    sync_groups_for_deleted_model(grants, &delete_retry, None, "dep-1", "gone")
        .await
        .unwrap();
    assert!(delete_retry.calls.lock().expect("calls").is_empty());
}

async fn invalid_row_after_commit(pool: &Pool, grants: &PostgresModelGrants<'_>) {
    execute(
        pool,
        r#"ALTER TABLE "LiteLLM_AccessGroupTable" DROP CONSTRAINT "LiteLLM_AccessGroupTable_pkey""#,
        &[],
    )
    .await;
    execute(
        pool,
        r#"ALTER TABLE "LiteLLM_AccessGroupTable" ALTER COLUMN "access_group_id" DROP NOT NULL"#,
        &[],
    )
    .await;
    insert_group(pool, None, Some(&expected(&["vanished"]))).await;
    let cache = ScriptedCache::new();
    let error = sync_groups_for_deleted_model(grants, &cache, None, "dep-1", "vanished")
        .await
        .expect_err("the returned id is NULL");
    assert!(matches!(error, ModelSyncError::Database(_)));
    assert!(cache.calls.lock().expect("calls").is_empty());
    let client = pool.get().await.expect("a fixture connection");
    let row = client
        .query_one(NULL_ID_NAMES_SQL, &[])
        .await
        .expect("the unnamed group row");
    assert_eq!(row.get::<_, Option<Vec<String>>>(0), Some(Vec::new()));
}
