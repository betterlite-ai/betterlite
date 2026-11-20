#![cfg(test)]

use std::sync::Arc;

use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_models::decisions::Json;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};
use clap::Parser;
use deadpool_postgres::tokio_postgres::NoTls;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::vector_store_registry::repository::VectorStoreDatabase as _;
use crate::api::vector_tool_selection::database::{
    PrismaVectorStoreSelection, VectorStoreSelectionDatabase as _,
};

use super::support::{object, text};

const STORES_TABLE: &str = r#"
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
    ('vs_known', 'bedrock', 'notes', NULL, NULL,
     '2026-02-03 04:05:06', '2026-02-03 04:05:06', NULL,
     '{"api_key": "inline"}'::jsonb),
    ('vs_other', 'openai', 'archive', 'older row', '{"tag": "x"}'::jsonb,
     '2026-02-01 00:00:00', '2026-02-01 00:00:00', 'cred', NULL);
"#;

const INDEXES_TABLE: &str = r#"
CREATE TABLE "LiteLLM_ManagedVectorStoreIndexTable" (
    "id" TEXT NOT NULL,
    "index_name" TEXT NOT NULL,
    "litellm_params" JSONB NOT NULL,
    "index_info" JSONB,
    "created_at" TIMESTAMP(3) NOT NULL,
    "updated_at" TIMESTAMP(3) NOT NULL,
    CONSTRAINT "LiteLLM_ManagedVectorStoreIndexTable_pkey" PRIMARY KEY ("id")
);
"#;

const INDEX_ROWS: &str = r#"
INSERT INTO "LiteLLM_ManagedVectorStoreIndexTable"
    ("id", "index_name", "litellm_params", "created_at", "updated_at")
VALUES
    ('i_new', 'notes',
     '{"vector_store_index": "idx-new", "vector_store_name": "notes"}'::jsonb,
     '2026-02-04 00:00:00', '2026-02-04 00:00:00'),
    ('i_old', 'archive',
     '{"vector_store_index": "idx-old", "vector_store_name": "archive"}'::jsonb,
     '2026-02-02 00:00:00', '2026-02-02 00:00:00');
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn test_url() -> Option<SecretString> {
    Environment::parse_from(["vector-store-selection"])
        .url
        .map(SecretString::from)
}

fn statements() -> Vec<String> {
    vec![
        STORES_TABLE.to_owned(),
        STORE_ROWS.to_owned(),
        INDEXES_TABLE.to_owned(),
        INDEX_ROWS.to_owned(),
    ]
}

async fn started_fixture() -> PostgresFixture {
    PostgresFixture::provision_with_statements(test_url().as_ref(), statements())
        .resolve()
        .await
        .expect("the fixture provisions")
}

fn session_store(fixture: &PostgresFixture) -> PostgresStore {
    let config = fixture
        .options
        .url
        .expose_secret()
        .parse()
        .expect("the fixture url parses");
    let manager = Manager::new(config, NoTls);
    PostgresStore {
        pool: Some(
            Pool::builder(manager)
                .max_size(2)
                .build()
                .expect("the pool builds"),
        ),
    }
}

fn no_publish() -> PublishChange<TableError> {
    Arc::new(|_object_type: &str| {
        let future: PublishFuture<TableError> = Box::pin(async { Ok::<(), TableError>(()) });
        future
    })
}

#[tokio::test]
async fn prisma_vector_store_selection_reads_the_single_row() {
    let fixture = started_fixture().await;
    let writer = session_store(&fixture);
    assert_seeded_rows(&writer).await;
    assert_absent_client().await;
    writer.pool.unwrap().close();
    fixture.close_async().await.unwrap();
}

async fn assert_seeded_rows(writer: &PostgresStore) {
    let availability = Availability::default();
    let client = PrismaClient {
        writer,
        reader: writer,
        availability: &availability,
    };
    let database = PrismaVectorStoreSelection::new(Some(client), no_publish());
    let found = database
        .find_store("vs_known")
        .await
        .unwrap()
        .expect("the seeded row");
    assert_eq!(found.vector_store_id.as_deref(), Some("vs_known"));
    assert_eq!(found.custom_llm_provider.as_deref(), Some("bedrock"));
    assert_eq!(found.vector_store_name.as_deref(), Some("notes"));
    assert_eq!(found.vector_store_description, None);
    assert_eq!(found.vector_store_metadata, None);
    assert_eq!(found.litellm_credential_name, None);
    assert!(found.created_at.is_some());
    assert_eq!(
        found.litellm_params,
        Some(
            serde_json::to_value(Json::Object(object(vec![("api_key", text("inline"))]))).unwrap()
        )
    );
    assert!(database.find_store("vs_absent").await.unwrap().is_none());
    let rows = database.stores().await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].vector_store_id.as_deref(), Some("vs_known"));
    assert_eq!(rows[1].vector_store_id.as_deref(), Some("vs_other"));
    let indexes = database.indexes().await.unwrap();
    assert_eq!(indexes.len(), 2);
    assert_eq!(indexes[0].id, "i_new");
    assert_eq!(indexes[1].index_name, "archive");
}

async fn assert_absent_client() {
    let absent = PrismaVectorStoreSelection::new(None, no_publish());
    let error = absent.find_store("vs_known").await.unwrap_err();
    assert!(matches!(error, TableError::Database(_)));
}
