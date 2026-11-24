use std::str::from_utf8;
use std::sync::Arc;

use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::schema_policy_resources::ADD_CLAUDE_CODE_PLUGIN_TABLE;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::NaiveDateTime;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use super::super::store::{ClaudePluginReadStore as _, PostgresPluginReadStore, StoredPluginRow};
use super::{ALL_FILTER, ENABLED_FILTER, at, granted_filter};

const INSERT_SQL: &str = r#"INSERT INTO "LiteLLM_ClaudeCodePluginTable" (id, name, version, description, manifest_json, enabled, created_at, updated_at, created_by) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct SeedRow<'text> {
    id: &'text str,
    name: &'text str,
    version: Option<&'text str>,
    description: Option<&'text str>,
    manifest: Option<&'text str>,
    enabled: bool,
    created: NaiveDateTime,
    updated: NaiveDateTime,
    created_by: Option<&'text str>,
}

async fn insert_row(pool: &Pool, seed: SeedRow<'_>) {
    let client = pool.get().await.expect("the pooled client");
    let parameters: [&(dyn ToSql + Sync); 9] = [
        &seed.id,
        &seed.name,
        &seed.version,
        &seed.description,
        &seed.manifest,
        &seed.enabled,
        &seed.created,
        &seed.updated,
        &seed.created_by,
    ];
    client
        .execute(INSERT_SQL, &parameters)
        .await
        .expect("the seeded row");
    drop(client);
}

async fn insert_rows(pool: &Pool) {
    insert_row(
        pool,
        SeedRow {
            id: "plugin-public-plugin",
            name: "public-plugin",
            version: Some("1.0.0"),
            description: Some("first"),
            manifest: Some(r#"{"name":"public-plugin"}"#),
            enabled: true,
            created: at(9, 0, 0).naive_utc(),
            updated: at(9, 5, 0).naive_utc(),
            created_by: Some("alice"),
        },
    )
    .await;
    insert_row(
        pool,
        SeedRow {
            id: "plugin-private-plugin",
            name: "private-plugin",
            version: None,
            description: None,
            manifest: None,
            enabled: false,
            created: at(9, 1, 0).naive_utc(),
            updated: at(9, 1, 0).naive_utc(),
            created_by: None,
        },
    )
    .await;
}

fn expected_rows() -> Vec<StoredPluginRow> {
    vec![
        StoredPluginRow {
            id: "plugin-private-plugin".to_owned(),
            name: "private-plugin".to_owned(),
            version: None,
            description: None,
            manifest_json: None,
            enabled: false,
            created_at: Some(at(9, 1, 0)),
            updated_at: Some(at(9, 1, 0)),
            created_by: None,
        },
        StoredPluginRow {
            id: "plugin-public-plugin".to_owned(),
            name: "public-plugin".to_owned(),
            version: Some("1.0.0".to_owned()),
            description: Some("first".to_owned()),
            manifest_json: Some(r#"{"name":"public-plugin"}"#.to_owned()),
            enabled: true,
            created_at: Some(at(9, 0, 0)),
            updated_at: Some(at(9, 5, 0)),
            created_by: Some("alice".to_owned()),
        },
    ]
}

fn sorted_names(rows: &[StoredPluginRow]) -> Vec<String> {
    let mut names: Vec<String> = rows.iter().map(|row| row.name.clone()).collect();
    names.sort();
    names
}

async fn database() -> (PostgresFixture, Pool, PostgresPluginReadStore) {
    let table = from_utf8(ADD_CLAUDE_CODE_PLUGIN_TABLE.bytes)
        .expect("the plugin table migration is UTF-8")
        .to_owned();
    let url = Environment::parse_from(["claude-plugin-read-tests"])
        .url
        .map(SecretString::from);
    let statements = vec![table];
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .expect("the postgres fixture");
    let settings = fixture
        .options
        .url
        .expose_secret()
        .parse()
        .expect("the fixture URL parses");
    let manager = Manager::new(settings, NoTls);
    let pool = Pool::builder(manager)
        .max_size(1)
        .build()
        .expect("the postgres pool");
    let store = PostgresPluginReadStore::new(Arc::new(PostgresStore {
        pool: Some(pool.clone()),
    }));
    (fixture, pool, store)
}

#[tokio::test]
async fn store_round_trip() {
    let (fixture, pool, store) = database().await;
    insert_rows(&pool).await;
    let mut all = store.list(&ALL_FILTER).await.expect("the rows");
    all.sort_by_key(|row| row.name.clone());
    assert_eq!(all, expected_rows());
    let enabled = store.list(&ENABLED_FILTER).await.expect("the enabled rows");
    assert_eq!(sorted_names(&enabled), vec!["public-plugin"]);
    let granted = store
        .list(&granted_filter(&["private-plugin"]))
        .await
        .expect("the granted rows");
    assert_eq!(
        sorted_names(&granted),
        vec!["private-plugin", "public-plugin"]
    );
    let ghost = store
        .list(&granted_filter(&["ghost-plugin"]))
        .await
        .expect("the granted rows");
    assert_eq!(sorted_names(&ghost), vec!["public-plugin"]);
    let found = store
        .find_by_name("public-plugin")
        .await
        .expect("the find")
        .expect("the row");
    assert_eq!(found, expected_rows()[1]);
    assert!(
        store
            .find_by_name("ghost-plugin")
            .await
            .expect("the find")
            .is_none()
    );
    pool.close();
    fixture.close_async().await.expect("the fixture closes");
}
