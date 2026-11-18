use std::sync::{Arc, Mutex};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_util::task::TaskTracker;

use crate::auth_read::include::{Including, IncludingQuery};
use crate::auth_read::table::ReadContext;
use crate::auth_resource_grants::managed_rows::{
    ManagedRowsQuery, ManagedVectorStoreRows, vector_store_rows_by_uuids,
};
use crate::auth_resource_grants::postgres::{open_permissions, open_vector_stores};
use crate::auth_resource_grants::rows::{ManagedVectorStoreTable, managed_store_list};
use crate::auth_resource_grants::tests::support::RecordingCache;
use crate::auth_resource_grants::vector_store::ObjectPermissionRows;

const SEED: &str = r#"
INSERT INTO "LiteLLM_ObjectPermissionTable" ("object_permission_id", "search_tools", "vector_stores")
    VALUES ('perm-db', ARRAY['tool-a'], ARRAY['vs-1']);
INSERT INTO "LiteLLM_ManagedVectorStoresTable"
    ("vector_store_id", "custom_llm_provider", "vector_store_name", "updated_at", "litellm_params")
    VALUES ('vs-1', 'openai', 'store one', CURRENT_TIMESTAMP, '{"litellm_credential_name":"cred-1"}'),
           ('vs-2', 'azure', 'store two', CURRENT_TIMESTAMP, NULL);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["auth-resource-grants-postgres-tests"])
        .url
        .map(SecretString::from);
    let statements = catalog_statements(&PACKAGED_CATALOGS).expect("the migration text");
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .expect("the database fixture");
    let pool = Pool::builder(Manager::new(
        fixture
            .options
            .url
            .expose_secret()
            .parse()
            .expect("the database url"),
        NoTls,
    ))
    .max_size(2)
    .build()
    .expect("the database pool");
    (fixture, pool)
}

async fn seed(pool: &Pool) {
    let client = pool.get().await.expect("the database connection");
    client.batch_execute(SEED).await.expect("the seeded rows");
}

async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.expect("the fixture teardown");
}

fn context() -> ReadContext<SystemClock> {
    ReadContext {
        default_deadline_seconds: 5.0,
        tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        tasks: TaskTracker::new(),
    }
}

#[test]
fn list_binds_the_id_array_and_take() {
    let filter = vec!["vs-1".to_owned()];
    let filtered = managed_store_list(Some(&filter), &Some(1));
    assert!(filtered.sql.contains("ANY($1::text[])"), "{}", filtered.sql);
    assert!(filtered.sql.contains("LIMIT $2"), "{}", filtered.sql);
    assert_eq!(filtered.parameters.len(), 2);
    let no_limit: Option<i64> = None;
    let bare = managed_store_list(None, &no_limit);
    assert!(!bare.sql.contains("ANY("), "{}", bare.sql);
    assert_eq!(bare.parameters.len(), 1);
    let listed = ManagedVectorStoreTable.including(
        IncludingQuery::Listed {
            filter: None,
            limit: &Some(1),
        },
        None,
    );
    assert!(listed.sql.contains("LIMIT $1"), "{}", listed.sql);
    assert_eq!(listed.parameters.len(), 1);
}

#[tokio::test]
async fn postgres_vector_rows() {
    let (fixture, pool) = database().await;
    seed(&pool).await;
    let client = Arc::new(pool.get().await.expect("the database connection"));
    let permissions = open_permissions(Arc::clone(&client), context());
    let stores = open_vector_stores(Arc::clone(&client), context());

    let permission = permissions
        .by_id("perm-db")
        .await
        .expect("the permission read")
        .expect("the stored permission");
    assert_eq!(
        permission.search_tools.0.as_deref(),
        Some(["tool-a".to_owned()].as_slice())
    );
    assert_eq!(
        permission.vector_stores.0.as_deref(),
        Some(["vs-1".to_owned()].as_slice())
    );
    assert!(
        permissions
            .by_id("perm-missing")
            .await
            .expect("the missing permission read")
            .is_none()
    );

    let one = stores
        .by_uuids(ManagedRowsQuery {
            uuids: vec!["vs-1".to_owned()],
            take: 1,
        })
        .await
        .expect("the one-store read");
    assert_eq!(one.len(), 1);
    let limited = stores
        .by_uuids(ManagedRowsQuery {
            uuids: vec!["vs-1".to_owned(), "vs-2".to_owned()],
            take: 1,
        })
        .await
        .expect("the limited read");
    assert_eq!(limited.len(), 1);
    assert!(
        stores
            .by_uuids(ManagedRowsQuery {
                uuids: vec!["vs-missing".to_owned()],
                take: 1,
            })
            .await
            .expect("the missing read")
            .is_empty()
    );

    let cache = RecordingCache::new(60.0);
    let projected = vector_store_rows_by_uuids(
        &["vs-1".to_owned(), "vs-2".to_owned()],
        true,
        &cache,
        &stores,
    )
    .await
    .expect("the projected lookup");
    let mut found: Vec<&str> = projected
        .iter()
        .map(|stored| stored.vector_store_id.as_str())
        .collect();
    found.sort_unstable();
    assert_eq!(found, ["vs-1", "vs-2"]);
    let first = projected
        .iter()
        .find(|stored| stored.vector_store_id == "vs-1")
        .expect("the seeded store");
    assert_eq!(first.custom_llm_provider, "openai");
    assert_eq!(first.vector_store_name.as_deref(), Some("store one"));
    let params = first.litellm_params.as_ref().expect("the stored params");
    assert_eq!(
        params.get("litellm_credential_name"),
        Some(&JsonData::String("cred-1".to_owned()))
    );
    assert_eq!(cache.writes().len(), 2);

    drop(stores);
    drop(permissions);
    drop(client);
    close(fixture, pool).await;
}
