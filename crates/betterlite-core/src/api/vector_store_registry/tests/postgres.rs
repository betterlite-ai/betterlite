#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::seed::FixtureSeed;
use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};
use clap::Parser;
use deadpool_postgres::GenericClient as _;
use deadpool_postgres::tokio_postgres::NoTls;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::vector_store_registry::repository::PrismaVectorStoreDatabase;
use crate::api::vector_store_registry::repository::VectorStoreDatabase as _;

const STORES_TABLE: &str = r#"
DROP TABLE IF EXISTS "LiteLLM_ManagedVectorStoresTable";
CREATE TABLE "LiteLLM_ManagedVectorStoresTable" (
    "vector_store_id" TEXT NOT NULL,
    "custom_llm_provider" TEXT NOT NULL,
    "vector_store_name" TEXT,
    "vector_store_description" TEXT,
    "vector_store_metadata" JSONB,
    "created_at" TIMESTAMP(3) NOT NULL,
    "updated_at" TIMESTAMP(3) NOT NULL,
    "litellm_credential_name" TEXT,
    "litellm_params" JSONB,
    "team_id" TEXT,
    "user_id" TEXT,
    CONSTRAINT "LiteLLM_ManagedVectorStoresTable_pkey" PRIMARY KEY ("vector_store_id")
);
"#;

const STORE_ROWS: &str = r#"
INSERT INTO "LiteLLM_ManagedVectorStoresTable"
    ("vector_store_id", "custom_llm_provider", "vector_store_name",
     "vector_store_description", "vector_store_metadata", "created_at", "updated_at",
     "litellm_credential_name", "litellm_params")
VALUES
    ('vs_old', 'openai', 'old', 'older row', '{"tag": "x"}'::jsonb,
     '2026-01-01T00:00:00', '2026-01-01T00:00:00', 'cred',
     '{"vector_store_id": "vs_old"}'::jsonb),
    ('vs_new', 'bedrock', 'new', NULL, NULL,
     '2026-01-02T00:00:00', '2026-01-02T00:00:00', NULL,
     '{"vector_store_id": "vs_new"}'::jsonb);
"#;

const INDEXES_TABLE: &str = r#"
DROP TABLE IF EXISTS "LiteLLM_ManagedVectorStoreIndexTable";
CREATE TABLE "LiteLLM_ManagedVectorStoreIndexTable" (
    "id" TEXT NOT NULL,
    "index_name" TEXT NOT NULL,
    "litellm_params" JSONB NOT NULL,
    "index_info" JSONB,
    "created_at" TIMESTAMP(3) NOT NULL,
    "created_by" TEXT,
    "updated_at" TIMESTAMP(3) NOT NULL,
    "updated_by" TEXT,
    CONSTRAINT "LiteLLM_ManagedVectorStoreIndexTable_pkey" PRIMARY KEY ("id"),
    CONSTRAINT "LiteLLM_ManagedVectorStoreIndexTable_index_name_key" UNIQUE ("index_name")
);
"#;

const INDEX_ROWS: &str = r#"
INSERT INTO "LiteLLM_ManagedVectorStoreIndexTable"
    ("id", "index_name", "litellm_params", "index_info", "created_at", "updated_at")
VALUES
    ('i-old', 'archive',
     '{"vector_store_index": "idx-old", "vector_store_name": "archive"}'::jsonb, NULL,
     '2026-01-01T00:00:00', '2026-01-01T00:00:00'),
    ('i-new', 'notes',
     '{"vector_store_index": "idx-new", "vector_store_name": "notes"}'::jsonb,
     '{"dimensions": 3}'::jsonb, '2026-01-02T00:00:00', '2026-01-02T00:00:00');
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn no_publish() -> PublishChange<TableError> {
    Arc::new(|_object_type: &str| {
        let future: PublishFuture<TableError> = Box::pin(async { Ok::<(), TableError>(()) });
        future
    })
}

fn test_url() -> Option<SecretString> {
    Environment::parse_from(["vector-store-registry"])
        .url
        .map(SecretString::from)
}

async fn provision() -> PostgresFixture {
    let mut generator = RandomDatabaseName;
    let mut task = PostgresFixture::provision_with_name_generator(
        test_url().as_ref(),
        &FixtureSeed::default(),
        &mut generator,
    );
    task.resolve().await.unwrap()
}

fn store(fixture: &PostgresFixture) -> PostgresStore {
    let config = fixture.options.url.expose_secret().parse().unwrap();
    let manager = Manager::new(config, NoTls);
    let pool = Pool::builder(manager).max_size(2).build().unwrap();
    PostgresStore { pool: Some(pool) }
}

async fn migrate(store: &PostgresStore) {
    let client = store.session().await.unwrap();
    client.batch_execute(STORES_TABLE).await.unwrap();
    client.batch_execute(STORE_ROWS).await.unwrap();
    client.batch_execute(INDEXES_TABLE).await.unwrap();
    client.batch_execute(INDEX_ROWS).await.unwrap();
}

#[tokio::test]
async fn prisma_vector_store_database_reads_rows() {
    let fixture = provision().await;
    let writer = store(&fixture);
    migrate(&writer).await;
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &writer,
        reader: &writer,
        availability: &availability,
    };
    let database = PrismaVectorStoreDatabase::new(Some(client), no_publish());

    store_rows_keep_the_database_order(&database).await;
    index_rows_keep_the_database_order(&database).await;
    drop(database);
    an_absent_client_reports_a_database_error().await;

    writer.pool.unwrap().close();
    fixture.close_async().await.unwrap();
}

async fn store_rows_keep_the_database_order(database: &PrismaVectorStoreDatabase<'_>) {
    let stores = database.stores().await.unwrap();
    assert_eq!(stores.len(), 2);
    let newest_store = stores.first().unwrap();
    assert_eq!(newest_store.vector_store_id.as_deref(), Some("vs_new"));
    assert_eq!(newest_store.custom_llm_provider.as_deref(), Some("bedrock"));
    assert_eq!(newest_store.vector_store_name.as_deref(), Some("new"));
    assert_eq!(newest_store.vector_store_metadata, None);
    assert_eq!(
        newest_store.litellm_params,
        Some(serde_json::json!({"vector_store_id": "vs_new"}))
    );
    let oldest_store = stores.get(1).expect("the older store row");
    assert_eq!(oldest_store.vector_store_id.as_deref(), Some("vs_old"));
    assert_eq!(
        oldest_store.vector_store_metadata,
        Some(serde_json::json!({"tag": "x"}))
    );
    assert_eq!(
        oldest_store.litellm_credential_name.as_deref(),
        Some("cred")
    );
    assert!(oldest_store.created_at.is_some());
}

async fn index_rows_keep_the_database_order(database: &PrismaVectorStoreDatabase<'_>) {
    let indexes = database.indexes().await.unwrap();
    assert_eq!(indexes.len(), 2);
    let newest_index = indexes.first().unwrap();
    assert_eq!(newest_index.id, "i-new");
    assert_eq!(newest_index.index_name, "notes");
    assert_eq!(newest_index.litellm_params.vector_store_index, "idx-new");
    assert_eq!(
        newest_index.index_info,
        Some(BTreeMap::from([(
            "dimensions".to_owned(),
            serde_json::json!(3)
        )]))
    );
    let oldest_index = indexes.get(1).expect("the older index row");
    assert_eq!(oldest_index.id, "i-old");
    assert_eq!(oldest_index.index_name, "archive");
    assert!(oldest_index.index_info.is_none());
}

async fn an_absent_client_reports_a_database_error() {
    let absent = PrismaVectorStoreDatabase::new(None, no_publish());
    let stores_error = absent.stores().await.unwrap_err();
    assert!(matches!(stores_error, TableError::Database(_)));
    let indexes_error = absent.indexes().await.unwrap_err();
    assert!(matches!(indexes_error, TableError::Database(_)));
}
