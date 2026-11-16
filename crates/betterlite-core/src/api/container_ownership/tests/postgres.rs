use std::collections::BTreeSet;
use std::sync::Arc;

use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_models::decisions::Json;
use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::seed::FixtureSeed;
use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};
use clap::Parser;
use deadpool_postgres::GenericClient as _;
use deadpool_postgres::tokio_postgres::NoTls;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::response_id_codecs::container::build_container_id;

use super::super::access::assert_container_access;
use super::super::errors::ContainerOwnershipError;
use super::super::forwarding::forwarding_params;
use super::super::list::allowed_container_ids;
use super::super::record::record_container_owner;
use super::super::repository::{
    ContainerObjectTable, ContainerOwnerUpdate, PrismaContainerObjects,
};
use super::super::response::synthetic_container;
use super::super::{
    CONTAINER_OBJECT_PURPOSE, ContainerCaller, ContainerOwnership, decode_for_ownership,
};
use super::support::{capture_logs, fixed_clock, principal};

const NATIVE_ID: &str = "container-native";

const MANAGED_OBJECT_TABLE: &str = r#"
DROP TABLE IF EXISTS "LiteLLM_ManagedObjectTable";
CREATE TABLE "LiteLLM_ManagedObjectTable" (
    "id" TEXT NOT NULL,
    "unified_object_id" TEXT NOT NULL,
    "model_object_id" TEXT NOT NULL,
    "file_object" JSONB NOT NULL,
    "file_purpose" TEXT NOT NULL,
    "created_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "created_by" TEXT,
    "updated_at" TIMESTAMP(3) NOT NULL,
    "updated_by" TEXT,
    CONSTRAINT "LiteLLM_ManagedObjectTable_pkey" PRIMARY KEY ("id")
);
CREATE UNIQUE INDEX "LiteLLM_ManagedObjectTable_unified_object_id_key"
    ON "LiteLLM_ManagedObjectTable" ("unified_object_id");
CREATE UNIQUE INDEX "LiteLLM_ManagedObjectTable_model_object_id_key"
    ON "LiteLLM_ManagedObjectTable" ("model_object_id");
"#;

const ROW_SQL: &str = r#"SELECT unified_object_id, file_purpose, created_by, updated_by,
    file_object->>'custom_llm_provider', file_object->>'provider_container_id'
    FROM "LiteLLM_ManagedObjectTable" WHERE model_object_id = $1"#;

type Ownership<'db> = ContainerOwnership<PrismaContainerObjects<'db>>;

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
    Environment::parse_from(["container-ownership"])
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
    client.batch_execute(MANAGED_OBJECT_TABLE).await.unwrap();
}

fn ownership<'db>(writer: &'db PostgresStore, availability: &'db Availability) -> Ownership<'db> {
    let client = PrismaClient {
        writer,
        reader: writer,
        availability,
    };
    ContainerOwnership::new(
        PrismaContainerObjects::new(Some(client), no_publish()),
        fixed_clock(),
    )
    .unwrap()
}

fn managed_container_id(model: &str) -> String {
    build_container_id(Some("openai"), Some(model), NATIVE_ID)
}

async fn record_container(ownership: &Ownership<'_>, model: &str) -> String {
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let managed = managed_container_id(model);
    record_container_owner(ownership, &synthetic_container(&managed), caller)
        .await
        .unwrap();
    managed
}

async fn assert_stored_row(writer: &PostgresStore, managed: &str) {
    let client = writer.session().await.unwrap();
    let row = client
        .query_one(ROW_SQL, &[&"container:openai:container-native"])
        .await
        .unwrap();
    let unified: String = row.try_get(0).unwrap();
    let purpose: String = row.try_get(1).unwrap();
    let created_by: Option<String> = row.try_get(2).unwrap();
    let updated_by: Option<String> = row.try_get(3).unwrap();
    let provider: Option<String> = row.try_get(4).unwrap();
    let provider_id: Option<String> = row.try_get(5).unwrap();
    assert_eq!(unified, managed);
    assert_eq!(purpose, CONTAINER_OBJECT_PURPOSE);
    assert_eq!(created_by.as_deref(), Some("alice"));
    assert_eq!(updated_by.as_deref(), Some("alice"));
    assert_eq!(provider.as_deref(), Some("openai"));
    assert_eq!(provider_id.as_deref(), Some(NATIVE_ID));
    let counted = client
        .query_one(
            r#"SELECT count(*) FROM "LiteLLM_ManagedObjectTable" WHERE model_object_id = $1"#,
            &[&"container:openai:container-native"],
        )
        .await
        .unwrap();
    let count: i64 = counted.try_get(0).unwrap();
    assert_eq!(count, 1);
}

async fn assert_native_forwarding(ownership: &Ownership<'_>) {
    let identity = decode_for_ownership(NATIVE_ID, "openai");
    let params = forwarding_params(ownership, NATIVE_ID, &identity)
        .await
        .unwrap();
    assert_eq!(params.container_id, NATIVE_ID);
    assert_eq!(params.model_id, Some("deployment-1".to_owned()));
}

async fn assert_access_checks(ownership: &Ownership<'_>, managed: &str) {
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let identity = assert_container_access(ownership, managed, caller)
        .await
        .unwrap();
    assert_eq!(identity.original_container_id, NATIVE_ID);
    let bob = principal("bob");
    let bob_caller = ContainerCaller {
        principal: Some(&bob),
        custom_llm_provider: "openai",
    };
    let error = assert_container_access(ownership, managed, bob_caller)
        .await
        .unwrap_err();
    assert!(matches!(&error, ContainerOwnershipError::Forbidden));
}

async fn assert_listed(ownership: &Ownership<'_>) {
    let alice = principal("alice");
    let allowed = allowed_container_ids(ownership, Some(&alice))
        .await
        .unwrap();
    assert_eq!(
        allowed,
        BTreeSet::from(["container:openai:container-native".to_owned()])
    );
    let bob = principal("bob");
    assert!(
        allowed_container_ids(ownership, Some(&bob))
            .await
            .unwrap()
            .is_empty()
    );
}

async fn insert_batch_row(writer: &PostgresStore) {
    let client = writer.session().await.unwrap();
    client
        .execute(
            r#"INSERT INTO "LiteLLM_ManagedObjectTable"
                (id, unified_object_id, model_object_id, file_object, file_purpose,
                 created_by, updated_by, updated_at)
                VALUES ($1, $2, $3, '{}'::jsonb, 'batch', 'alice', 'alice', now())"#,
            &[
                &"row-batch",
                &"container-other",
                &"container:openai:container-other",
            ],
        )
        .await
        .unwrap();
}

async fn assert_other_purpose_is_untracked(ownership: &Ownership<'_>) {
    let objects = &ownership.objects;
    let found = objects
        .find_unique("container:openai:container-other")
        .await
        .unwrap();
    assert_eq!(found.unwrap().file_purpose, "batch");
    assert!(
        objects
            .find_container("container:openai:container-other")
            .await
            .unwrap()
            .is_none()
    );
    let owned = objects.find_owned(vec!["alice".to_owned()]).await.unwrap();
    assert_eq!(owned, vec!["container:openai:container-native".to_owned()]);
}

async fn assert_conflicting_purpose(ownership: &Ownership<'_>) {
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let error = record_container_owner(ownership, &synthetic_container("container-other"), caller)
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        ContainerOwnershipError::ConflictingPurpose
    ));
    assert_eq!(error.status_code(), 500);
}

async fn assert_forbidden_update(ownership: &Ownership<'_>) {
    let bob = principal("bob");
    let caller = ContainerCaller {
        principal: Some(&bob),
        custom_llm_provider: "openai",
    };
    let managed = managed_container_id("deployment-3");
    let error = record_container_owner(ownership, &synthetic_container(&managed), caller)
        .await
        .unwrap_err();
    assert!(matches!(&error, ContainerOwnershipError::Forbidden));
}

async fn assert_missing_row(writer: &PostgresStore, availability: &Availability) {
    let client = PrismaClient {
        writer,
        reader: writer,
        availability,
    };
    let objects = PrismaContainerObjects::new(Some(client), no_publish());
    let error = objects
        .update(ContainerOwnerUpdate {
            model_object_id: "container:openai:missing".to_owned(),
            unified_object_id: "container-missing".to_owned(),
            file_object: Json::Null,
            updated_by: "alice".to_owned(),
        })
        .await
        .unwrap_err();
    assert!(matches!(error, TableError::MissingRow));
}

#[tokio::test]
async fn postgres_owner_roundtrip() {
    let fixture = provision().await;
    let writer = store(&fixture);
    migrate(&writer).await;
    let availability = Availability::default();
    let first = ownership(&writer, &availability);
    let managed = record_container(&first, "deployment-1").await;
    assert_stored_row(&writer, &managed).await;
    drop(first);
    let second = ownership(&writer, &availability);
    assert_native_forwarding(&second).await;
    assert_access_checks(&second, &managed).await;
    assert_listed(&second).await;
    drop(second);
    let third = ownership(&writer, &availability);
    let updated = record_container(&third, "deployment-2").await;
    drop(third);
    assert_stored_row(&writer, &updated).await;
    insert_batch_row(&writer).await;
    let fourth = ownership(&writer, &availability);
    assert_other_purpose_is_untracked(&fourth).await;
    assert_conflicting_purpose(&fourth).await;
    assert_forbidden_update(&fourth).await;
    drop(fourth);
    assert_missing_row(&writer, &availability).await;
    writer.pool.unwrap().close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn prisma_objects_without_a_client_skip_recording() {
    let ownership = ContainerOwnership::new(
        PrismaContainerObjects::new(None, no_publish()),
        fixed_clock(),
    )
    .unwrap();
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let (logs, _guard) = capture_logs();
    record_container_owner(&ownership, &synthetic_container("container-1"), caller)
        .await
        .unwrap();
    let warnings = logs.warnings();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("prisma_client is None"));
}
