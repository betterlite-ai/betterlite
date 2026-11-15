use std::str::from_utf8;
use std::sync::Arc;

use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::{
    schema_agent_identity, schema_baseline, schema_budget_windows, schema_daily_usage,
    schema_identity_history, schema_managed_resources, schema_policy_resources,
    schema_routing_state,
};
use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::seed::FixtureSeed;
use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};
use chrono::NaiveDateTime;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use crate::loader::startup::process_environment;
use crate::sections::mcp_toolset_storage::sql::MCPToolsetRepository;
use crate::sections::mcp_toolset_storage::{
    MCPToolsetStorageError, NewMCPToolsetRequest, UpdateMCPToolsetRequest, create_mcp_toolset,
    delete_mcp_toolset, get_mcp_toolset, get_mcp_toolset_by_name, list_mcp_toolsets,
    update_mcp_toolset,
};

use super::tool;

fn configured_url() -> Option<SecretString> {
    process_environment()
        .get("POSTGRES_TEST_URL")
        .cloned()
        .map(SecretString::from)
}

fn no_publish() -> PublishChange<TableError> {
    Arc::new(|_object_type: &str| -> PublishFuture<TableError> { Box::pin(async { Ok(()) }) })
}

struct Case {
    fixture: PostgresFixture,
    store: PostgresStore,
    availability: Availability,
}

impl Case {
    async fn new() -> Self {
        let url = configured_url();
        let mut generator = RandomDatabaseName;
        let mut task = PostgresFixture::provision_with_name_generator(
            url.as_ref(),
            &FixtureSeed::default(),
            &mut generator,
        );
        let fixture = task.resolve().await.unwrap();
        let config = fixture.options.url.expose_secret().parse().unwrap();
        let pool = Pool::builder(Manager::new(config, NoTls))
            .max_size(2)
            .build()
            .unwrap();
        let store = PostgresStore { pool: Some(pool) };
        migrate(&store).await;
        Self {
            fixture,
            store,
            availability: Availability::default(),
        }
    }

    fn repository(&self) -> MCPToolsetRepository<'_> {
        let client = PrismaClient {
            writer: &self.store,
            reader: &self.store,
            availability: &self.availability,
        };
        MCPToolsetRepository {
            client: Some(client),
            publish: no_publish(),
        }
    }

    async fn execute(&self, sql: &str, params: &[&(dyn ToSql + Sync)]) {
        let client = self.store.session().await.unwrap();
        client.execute(sql, params).await.unwrap();
    }

    async fn close(self) {
        self.store.pool.unwrap().close();
        self.fixture.close_async().await.unwrap();
    }
}

async fn migrate(store: &PostgresStore) {
    let client = store.session().await.unwrap();
    client
        .batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
        .await
        .unwrap();
    for catalog in [
        schema_baseline::MIGRATION_CATALOG,
        schema_daily_usage::MIGRATION_CATALOG,
        schema_managed_resources::MIGRATION_CATALOG,
        schema_identity_history::MIGRATION_CATALOG,
        schema_policy_resources::MIGRATION_CATALOG,
        schema_routing_state::MIGRATION_CATALOG,
        schema_budget_windows::MIGRATION_CATALOG,
        schema_agent_identity::MIGRATION_CATALOG,
    ] {
        for migration in catalog.files {
            client
                .batch_execute(from_utf8(migration.bytes).unwrap())
                .await
                .unwrap();
        }
    }
}

#[tokio::test]
async fn postgres_round_trip_keeps_every_field() {
    let case = Case::new().await;
    let repository = case.repository();
    let request: NewMCPToolsetRequest = serde_json::from_str(
        r#"{"toolset_name": "alpha", "description": "first", "tools": [{"server_id": "s1", "tool_name": "t1"}]}"#,
    )
    .unwrap();
    let created = create_mcp_toolset(&repository, &request, "creator")
        .await
        .unwrap();
    assert_eq!(created.tools, vec![tool("s1", "t1")]);
    assert_eq!(created.created_by.as_deref(), Some("creator"));
    assert_eq!(created.updated_by.as_deref(), Some("creator"));
    assert!(created.created_at.is_some());
    assert!(created.updated_at.is_some());
    let loaded = get_mcp_toolset(&repository, &created.toolset_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded, created);
    let by_name = get_mcp_toolset_by_name(&repository, "alpha")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(by_name.toolset_id, created.toolset_id);
    assert_eq!(
        list_mcp_toolsets(&repository, None, false).await.unwrap(),
        vec![created]
    );
    case.close().await;
}

#[tokio::test]
async fn postgres_lists_only_the_requested_ids() {
    let case = Case::new().await;
    let repository = case.repository();
    let alpha: NewMCPToolsetRequest = serde_json::from_str(r#"{"toolset_name": "alpha"}"#).unwrap();
    create_mcp_toolset(&repository, &alpha, "creator")
        .await
        .unwrap();
    let payload: NewMCPToolsetRequest =
        serde_json::from_str(r#"{"toolset_name": "beta"}"#).unwrap();
    let beta = create_mcp_toolset(&repository, &payload, "creator")
        .await
        .unwrap();
    let ids = vec![beta.toolset_id.clone()];
    let listed = list_mcp_toolsets(&repository, Some(&ids), false)
        .await
        .unwrap();
    assert_eq!(listed, vec![beta.clone()]);
    let fetched = get_mcp_toolset(&repository, &beta.toolset_id)
        .await
        .unwrap();
    assert_eq!(fetched, Some(beta.clone()));
    let by_name = get_mcp_toolset_by_name(&repository, "beta").await.unwrap();
    assert_eq!(by_name, Some(beta.clone()));
    assert_eq!(
        list_mcp_toolsets(&repository, Some(&ids), true)
            .await
            .unwrap(),
        vec![beta.clone()]
    );
    assert_eq!(
        list_mcp_toolsets(&repository, None, true)
            .await
            .unwrap()
            .len(),
        2
    );
    case.close().await;
}

#[tokio::test]
async fn postgres_lists_from_the_writer_when_asked() {
    let case = Case::new().await;
    let missing_reader = PostgresStore { pool: None };
    let client = PrismaClient {
        writer: &case.store,
        reader: &missing_reader,
        availability: &case.availability,
    };
    let repository = MCPToolsetRepository {
        client: Some(client),
        publish: no_publish(),
    };
    let request: NewMCPToolsetRequest =
        serde_json::from_str(r#"{"toolset_name": "alpha"}"#).unwrap();
    create_mcp_toolset(&repository, &request, "creator")
        .await
        .unwrap();
    assert!(
        list_mcp_toolsets(&repository, None, false)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        list_mcp_toolsets(&repository, None, true)
            .await
            .unwrap()
            .len(),
        1
    );
    case.close().await;
}

#[tokio::test]
async fn postgres_update_keeps_unset_fields_and_clears_the_description() {
    let case = Case::new().await;
    let repository = case.repository();
    let request: NewMCPToolsetRequest = serde_json::from_str(
        r#"{"toolset_name": "alpha", "description": "first", "tools": [{"server_id": "s1", "tool_name": "t1"}]}"#,
    )
    .unwrap();
    let created = create_mcp_toolset(&repository, &request, "creator")
        .await
        .unwrap();
    case.execute(
        "UPDATE \"LiteLLM_MCPToolsetTable\" SET updated_at = '2020-01-01 00:00:00' WHERE toolset_id = $1",
        &[&created.toolset_id],
    )
    .await;
    let payload = format!(
        r#"{{"toolset_id": "{}", "description": null}}"#,
        created.toolset_id
    );
    let changes: UpdateMCPToolsetRequest = serde_json::from_str(&payload).unwrap();
    let updated = update_mcp_toolset(&repository, &changes, "editor")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.toolset_name, "alpha");
    assert_eq!(updated.tools, vec![tool("s1", "t1")]);
    assert_eq!(updated.description, None);
    assert_eq!(updated.updated_by.as_deref(), Some("editor"));
    let pinned = NaiveDateTime::parse_from_str("2020-01-01T00:00:00", "%Y-%m-%dT%H:%M:%S").unwrap();
    assert!(updated.updated_at.unwrap() > pinned.and_utc());
    case.close().await;
}

#[tokio::test]
async fn postgres_update_empties_the_tools_without_touching_another_toolset() {
    let case = Case::new().await;
    let repository = case.repository();
    let request: NewMCPToolsetRequest = serde_json::from_str(
        r#"{"toolset_name": "alpha", "tools": [{"server_id": "s1", "tool_name": "t1"}]}"#,
    )
    .unwrap();
    let created = create_mcp_toolset(&repository, &request, "creator")
        .await
        .unwrap();
    let beta: NewMCPToolsetRequest = serde_json::from_str(
        r#"{"toolset_name": "beta", "description": "kept", "tools": [{"server_id": "s2", "tool_name": "t2"}]}"#,
    )
    .unwrap();
    let other = create_mcp_toolset(&repository, &beta, "creator")
        .await
        .unwrap();
    let payload = format!(r#"{{"toolset_id": "{}", "tools": []}}"#, created.toolset_id);
    let changes: UpdateMCPToolsetRequest = serde_json::from_str(&payload).unwrap();
    let emptied = update_mcp_toolset(&repository, &changes, "editor")
        .await
        .unwrap()
        .unwrap();
    assert!(emptied.tools.is_empty());
    assert_eq!(emptied.toolset_id, created.toolset_id);
    let untouched = get_mcp_toolset(&repository, &other.toolset_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(untouched.toolset_name, "beta");
    assert_eq!(untouched.description.as_deref(), Some("kept"));
    assert_eq!(untouched.tools, vec![tool("s2", "t2")]);
    case.close().await;
}

#[tokio::test]
async fn postgres_delete_removes_the_row_once() {
    let case = Case::new().await;
    let repository = case.repository();
    let request: NewMCPToolsetRequest =
        serde_json::from_str(r#"{"toolset_name": "alpha"}"#).unwrap();
    let created = create_mcp_toolset(&repository, &request, "creator")
        .await
        .unwrap();
    let payload: NewMCPToolsetRequest =
        serde_json::from_str(r#"{"toolset_name": "beta"}"#).unwrap();
    let other = create_mcp_toolset(&repository, &payload, "creator")
        .await
        .unwrap();
    let removed = delete_mcp_toolset(&repository, &created.toolset_id)
        .await
        .unwrap();
    assert_eq!(removed, Some(created.clone()));
    assert!(
        get_mcp_toolset(&repository, &other.toolset_id)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        delete_mcp_toolset(&repository, &created.toolset_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        get_mcp_toolset(&repository, &created.toolset_id)
            .await
            .unwrap()
            .is_none()
    );
    case.close().await;
}

#[tokio::test]
async fn postgres_stores_the_tools_column_as_a_json_string() {
    let case = Case::new().await;
    let repository = case.repository();
    let request: NewMCPToolsetRequest = serde_json::from_str(
        r#"{"toolset_name": "alpha", "tools": [{"server_id": "s1", "tool_name": "t1"}]}"#,
    )
    .unwrap();
    create_mcp_toolset(&repository, &request, "creator")
        .await
        .unwrap();
    let client = case.store.session().await.unwrap();
    let row = client
        .query_one(
            "SELECT tools::text, jsonb_typeof(tools) FROM \"LiteLLM_MCPToolsetTable\"",
            &[],
        )
        .await
        .unwrap();
    drop(client);
    let stored: String = row.get(0);
    let kind: String = row.get(1);
    assert_eq!(kind, "string");
    assert_eq!(stored, r#""[{\"server_id\":\"s1\",\"tool_name\":\"t1\"}]""#);
    case.close().await;
}

#[tokio::test]
async fn postgres_surfaces_a_query_error_as_a_storage_error() {
    let case = Case::new().await;
    case.execute("DROP TABLE \"LiteLLM_MCPToolsetTable\"", &[])
        .await;
    let error = get_mcp_toolset(&case.repository(), "id-1")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        MCPToolsetStorageError::Table(TableError::Database(_))
    ));
    case.close().await;
}

#[tokio::test]
async fn postgres_reads_a_tools_column_stored_as_a_json_array() {
    let case = Case::new().await;
    let repository = case.repository();
    case.execute(
        "INSERT INTO \"LiteLLM_MCPToolsetTable\" (toolset_id, toolset_name, tools) VALUES ($1, $2, $3::text::jsonb)",
        &[&"id-array", &"alpha", &r#"[{"server_id": "s1", "tool_name": "t1"}]"#],
    )
    .await;
    let loaded = get_mcp_toolset(&repository, "id-array")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.tools, vec![tool("s1", "t1")]);
    case.close().await;
}
