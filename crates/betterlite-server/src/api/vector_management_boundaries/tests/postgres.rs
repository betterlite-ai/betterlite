use std::sync::Arc;

use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::{Config, NoTls};

use crate::api::vector_management_boundaries::rows::{
    ManagedVectorStoreTable, VectorStoreRows as _,
};

// the table LiteLLM's Prisma model declares, without a column for is_config
const STORAGE: &str = r#"CREATE TABLE "LiteLLM_ManagedVectorStoresTable" (
    "vector_store_id" TEXT NOT NULL,
    "custom_llm_provider" TEXT NOT NULL,
    "vector_store_name" TEXT,
    "vector_store_description" TEXT,
    "vector_store_metadata" JSONB,
    "created_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(3) NOT NULL,
    "litellm_credential_name" TEXT,
    "litellm_params" JSONB,
    "team_id" TEXT,
    "user_id" TEXT,
    CONSTRAINT "LiteLLM_ManagedVectorStoresTable_pkey" PRIMARY KEY ("vector_store_id")
);
INSERT INTO "LiteLLM_ManagedVectorStoresTable"
    ("vector_store_id", "custom_llm_provider", "vector_store_name", "vector_store_metadata",
     "updated_at", "litellm_credential_name", "litellm_params", "team_id", "user_id")
VALUES
    ('vs_a', 'openai', 'Store', '{"env": "prod"}'::jsonb, '2026-01-01T00:00:00', 'cred',
     '{"api_key": "secret", "region": "us"}'::jsonb, 'team_a', 'user_a'),
    ('vs_b', 'bedrock', 'Other', NULL, '2026-01-02T00:00:00', NULL, NULL, NULL, NULL);"#;

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

#[tokio::test]
async fn find_unique_reads_one_managed_vector_store_row() {
    let url = Environment::parse_from(["vector-management-boundaries"])
        .url
        .map(SecretString::from);
    let fixture =
        PostgresFixture::provision_with_statements(url.as_ref(), vec![STORAGE.to_owned()])
            .resolve()
            .await
            .unwrap();
    let config: Config = fixture.options.url.expose_secret().parse().unwrap();
    let pool = Pool::builder(Manager::new(config, NoTls))
        .max_size(2)
        .build()
        .unwrap();
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &store,
        reader: &store,
        availability: &availability,
    };
    let table = ManagedVectorStoreTable::new(Some(client), no_publish());

    let found = table
        .find_unique("vs_a")
        .await
        .unwrap()
        .expect("vs_a is stored");
    assert_eq!(found.vector_store_id, "vs_a");
    assert_eq!(found.custom_llm_provider, "openai");
    assert_eq!(found.vector_store_name.as_deref(), Some("Store"));
    assert!(found.vector_store_description.is_none());
    assert!(found.vector_store_metadata.is_some());
    assert!(found.created_at.is_some());
    assert_eq!(found.litellm_credential_name.as_deref(), Some("cred"));
    assert!(found.litellm_params.is_some());
    assert_eq!(found.team_id.as_deref(), Some("team_a"));
    assert_eq!(found.user_id.as_deref(), Some("user_a"));
    assert!(!found.is_config);
    assert!(table.find_unique("vs_missing").await.unwrap().is_none());

    drop(table);
    pool.close();
    fixture.close_async().await.unwrap();
}
