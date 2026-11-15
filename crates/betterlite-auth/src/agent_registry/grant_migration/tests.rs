#![cfg(test)]

use std::time::Duration;

use betterlite_db::postgres_store::session::PostgresStore;
use deadpool_postgres::Transaction;
use tokio::time::sleep;

use crate::agent_registry::AgentRegistry;
use crate::agent_registry::AgentRegistryApi as _;
use crate::agent_registry::AgentRegistryError;
use crate::agent_registry::GrantMigrationResult;
use crate::agent_registry::tests::support::{
    ALPHA_ENTRY, ALPHA_LEGACY, ALPHA_STABLE, BETA_ENTRY, BETA_LEGACY, BETA_STABLE, SCHEMA,
    TestDatabase, agent, insert_row, loaded, row_agents,
};

#[tokio::test]
async fn without_legacy_ids_the_migration_skips_the_database() {
    let registry = AgentRegistry::default();
    registry.register_agent(agent("db-only", "db-only"));
    let store = PostgresStore { pool: None };
    let result = registry.migrate_legacy_grant_ids(&store).await.unwrap();
    assert_eq!(
        result,
        GrantMigrationResult {
            rewritten: 0,
            missed: 0
        }
    );
}

#[tokio::test]
async fn rows_with_legacy_ids_are_rewritten_in_first_seen_order() {
    let db = TestDatabase::start(SCHEMA).await;
    let first = vec![ALPHA_LEGACY.to_owned(), String::from("external")];
    insert_row(&db.pool, "p1", &first).await;
    let second = vec![
        BETA_LEGACY.to_owned(),
        ALPHA_STABLE.to_owned(),
        String::from("unknown-id"),
        ALPHA_LEGACY.to_owned(),
    ];
    insert_row(&db.pool, "p2", &second).await;
    insert_row(&db.pool, "p3", &[String::from("unrelated")]).await;
    let registry = loaded(&[ALPHA_ENTRY, BETA_ENTRY]);
    let result = registry
        .migrate_legacy_grant_ids(&db.store())
        .await
        .unwrap();
    assert_eq!(
        result,
        GrantMigrationResult {
            rewritten: 2,
            missed: 0
        }
    );
    assert_eq!(
        row_agents(&db.pool, "p1").await,
        Some(vec![ALPHA_STABLE.to_owned(), String::from("external")])
    );
    assert_eq!(
        row_agents(&db.pool, "p2").await,
        Some(vec![
            BETA_STABLE.to_owned(),
            ALPHA_STABLE.to_owned(),
            String::from("unknown-id"),
        ])
    );
    assert_eq!(
        row_agents(&db.pool, "p3").await,
        Some(vec![String::from("unrelated")])
    );
    let second_run = registry
        .migrate_legacy_grant_ids(&db.store())
        .await
        .unwrap();
    assert_eq!(
        second_run,
        GrantMigrationResult {
            rewritten: 0,
            missed: 0
        }
    );
    db.close().await;
}

#[tokio::test]
async fn a_database_error_is_returned() {
    let db = TestDatabase::start("SELECT 1;").await;
    let registry = loaded(&[ALPHA_ENTRY]);
    let error = registry
        .migrate_legacy_grant_ids(&db.store())
        .await
        .unwrap_err();
    assert!(matches!(error, AgentRegistryError::Database(_)));
    db.close().await;
}

#[tokio::test]
async fn a_row_changed_between_the_read_and_the_write_is_missed() {
    let db = TestDatabase::start(SCHEMA).await;
    insert_row(&db.pool, "p1", &[ALPHA_LEGACY.to_owned()]).await;
    let registry = loaded(&[ALPHA_ENTRY]);
    let store = db.store();
    let mut client = db.pool.get().await.unwrap();
    let transaction = client.transaction().await.unwrap();
    transaction
        .execute(
            r#"SELECT object_permission_id FROM "LiteLLM_ObjectPermissionTable" WHERE object_permission_id = $1 FOR UPDATE"#,
            &[&"p1"],
        )
        .await
        .unwrap();
    let editor = async move {
        wait_for_blocked_update(&transaction).await;
        let concurrent = vec![ALPHA_LEGACY.to_owned(), String::from("concurrent")];
        transaction
            .execute(
                r#"UPDATE "LiteLLM_ObjectPermissionTable" SET agents = $2 WHERE object_permission_id = $1"#,
                &[&"p1", &concurrent],
            )
            .await
            .unwrap();
        transaction.commit().await.unwrap();
    };
    let migration = registry.migrate_legacy_grant_ids(&store);
    let (result, ()) = tokio::join!(migration, editor);
    let result = result.unwrap();
    assert_eq!(
        result,
        GrantMigrationResult {
            rewritten: 0,
            missed: 1
        }
    );
    assert_eq!(
        row_agents(&db.pool, "p1").await,
        Some(vec![ALPHA_LEGACY.to_owned(), String::from("concurrent")])
    );
    db.close().await;
}

async fn wait_for_blocked_update(transaction: &Transaction<'_>) {
    for _ in 0..1000 {
        let row = transaction
            .query_one(
                "SELECT count(*) FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND query LIKE 'UPDATE \"LiteLLM_ObjectPermissionTable\"%'",
                &[],
            )
            .await
            .unwrap();
        let waiting: i64 = row.get(0);
        if waiting > 0 {
            return;
        }
        sleep(Duration::from_millis(10)).await;
    }
    panic!("the migration update never waited on the row lock");
}
