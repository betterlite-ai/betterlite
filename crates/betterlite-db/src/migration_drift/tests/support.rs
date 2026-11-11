use rstest::rstest;
use secrecy::SecretString;
use tokio_postgres::Client as PostgresClient;
use tracing::dispatcher::set_default;

use super::MigrationDriftCheck;
use super::ahead::{hostile_names, shown, warn_if_db_ahead_of_head};
use super::comparison::{SchemaCheck, check_schema, evaluate, summary};
use super::fixtures::{
    CapturedLogs, PUBLIC_SCHEMA, READ_TIMEOUT, TestDatabase, apply_compiled, apply_pinned, capture,
    read_contract, reset_public, with_databases, with_schema,
};
use super::guard::{LensGuardError, raise_if_lens_rename_pending, spend_logs_is_partitioned};
use super::index_repair::{
    INDEX_REPAIR_LOCK_KEY, InvalidIndex, index_repair, invalid_litellm_indexes,
    repair_invalid_indexes, repair_under_migration_lock,
};
use crate::migration_ledger::environment::prisma_schema_param;
use crate::migration_ledger::lock::MIGRATION_LOCK_KEY;
use crate::schema_agent_identity::MIGRATION_CATALOG as B709;
use crate::schema_contract::catalog::Attribute;
use crate::schema_contract::comparison::{Change, Comparison};
use crate::schema_contract::{Fact, ObjectKind, SchemaContract};

const LEDGER_DDL: &str = "CREATE TABLE _prisma_migrations (id varchar(36) PRIMARY KEY NOT NULL, \
    checksum varchar(64) NOT NULL, finished_at timestamptz, migration_name varchar(255) NOT NULL, \
    logs text, rolled_back_at timestamptz, started_at timestamptz NOT NULL DEFAULT now(), \
    applied_steps_count integer NOT NULL DEFAULT 0)";

fn b709_names() -> Vec<String> {
    B709.files
        .iter()
        .map(|file| file.directory.to_owned())
        .collect()
}

fn fact(kind: ObjectKind, relation: &str, member: &str, attribute: Attribute) -> Fact {
    Fact {
        kind,
        relation: relation.to_owned(),
        member: member.to_owned(),
        attribute,
    }
}

fn contract(facts: Vec<Fact>) -> SchemaContract {
    SchemaContract {
        facts: facts
            .into_iter()
            .map(|fact| (fact, Some("value".to_owned())))
            .collect(),
    }
}

fn base_contract() -> SchemaContract {
    contract(vec![fact(
        ObjectKind::Relation,
        "LiteLLM_Base",
        "",
        Attribute::Kind,
    )])
}

fn relation_present(contract: &SchemaContract, name: &str) -> bool {
    contract
        .facts
        .keys()
        .any(|fact| fact.kind == ObjectKind::Relation && fact.relation == name)
}

#[derive(Clone, Copy)]
enum Presence {
    Missing,
    Extra,
}

fn changed(
    comparison: &Comparison,
    kind: ObjectKind,
    relation: &str,
    member: &str,
    presence: Presence,
) -> bool {
    comparison.differences.iter().any(|(entry, change)| {
        entry.kind == kind
            && entry.relation == relation
            && entry.member == member
            && match (presence, change) {
                (Presence::Missing, Change::Missing { .. })
                | (Presence::Extra, Change::Extra { .. }) => true,
                (Presence::Missing | Presence::Extra, _) => false,
            }
    })
}

fn index_names(indexes: &[InvalidIndex]) -> Vec<&str> {
    indexes.iter().map(|index| index.name.as_str()).collect()
}

async fn ahead_unavailable_is_silent(known: &[String]) {
    let absent = MigrationDriftCheck {
        database_url: None,
        direct_url: None,
    };
    assert!(warn_if_db_ahead_of_head(&absent, known).await.is_empty());
    let malformed = MigrationDriftCheck {
        database_url: Some(SecretString::from("postgresql://[::1/db")),
        direct_url: None,
    };
    assert!(warn_if_db_ahead_of_head(&malformed, known).await.is_empty());
    let unreachable = MigrationDriftCheck {
        database_url: Some(SecretString::from(
            "postgresql://postgres:postgres@127.0.0.1:1/postgres",
        )),
        direct_url: None,
    };
    assert!(
        warn_if_db_ahead_of_head(&unreachable, known)
            .await
            .is_empty()
    );
}

async fn assert_repair_result(client: &PostgresClient, schema: &str, captured: &str) {
    assert!(captured.contains("found invalid indexes"), "{captured}");
    assert!(
        captured.contains("LiteLLM_aaa_repairable (table size"),
        "{captured}"
    );
    assert!(captured.contains("Dropped leftover"), "{captured}");
    assert!(captured.contains("Rebuilt"), "{captured}");
    let remaining = invalid_litellm_indexes(client, schema).await.unwrap();
    assert!(remaining.is_empty());
    assert_eq!(
        index_valid(client, schema, "LiteLLM_aaa_repairable").await,
        Some(true)
    );
    assert_eq!(
        index_valid(client, schema, "LiteLLM_ccc_leftover_ccnew1").await,
        None
    );
    assert_eq!(index_valid(client, schema, "other_idx").await, Some(false));
}

async fn schema_check(
    database: &TestDatabase,
    expected: &SchemaContract,
    disabled: bool,
) -> SchemaCheck {
    check_schema(
        &database.check(),
        disabled,
        Some(expected),
        &database.schema,
        READ_TIMEOUT,
    )
    .await
}

async fn index_valid(client: &PostgresClient, schema: &str, name: &str) -> Option<bool> {
    client
        .query_opt(
            "SELECT i.indisvalid FROM pg_index i \
             JOIN pg_class c ON c.oid = i.indexrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relname = $2",
            &[&schema, &name],
        )
        .await
        .unwrap()
        .map(|row| row.get(0))
}

async fn create_ledger(client: &PostgresClient) {
    client.batch_execute(LEDGER_DDL).await.unwrap();
}

async fn insert_ledger(client: &PostgresClient) {
    client
        .batch_execute(
            "INSERT INTO _prisma_migrations (id, checksum, migration_name, started_at, finished_at, rolled_back_at) VALUES \
             ('row-known', 'checksum', '20261001100000_rename_lens', now(), now(), NULL), \
             ('row-same', 'checksum', '20261001100000_parallel', now(), now(), NULL), \
             ('row-newer', 'checksum', '20261101000000_future', now(), now(), NULL), \
             ('row-older', 'checksum', '20200101000000_old', now(), now(), NULL), \
             ('row-rolled-back', 'checksum', '20261201000000_rolled_back', now(), now(), now()), \
             ('row-unfinished', 'checksum', '20261301000000_unfinished', now(), NULL, NULL), \
             ('row-unexpected', 'checksum', 'unexpected_name', now(), now(), NULL)",
        )
        .await
        .unwrap();
}

async fn duplicate_table(client: &PostgresClient, table: &str) {
    client
        .batch_execute(&format!(
            "CREATE TABLE \"{table}\" (value integer); INSERT INTO \"{table}\" VALUES (1), (1)"
        ))
        .await
        .unwrap();
}

// the failed concurrent build leaves the index INVALID, which is exactly the state the repair code recovers
async fn invalid_unique_index(client: &PostgresClient, table: &str, index: &str) {
    let statement = format!("CREATE UNIQUE INDEX CONCURRENTLY \"{index}\" ON \"{table}\" (value)");
    client.batch_execute(&statement).await.unwrap_err();
}

async fn deduplicate(client: &PostgresClient, table: &str) {
    let statement =
        format!("DELETE FROM \"{table}\" WHERE ctid NOT IN (SELECT min(ctid) FROM \"{table}\")");
    client.batch_execute(&statement).await.unwrap();
}

async fn create_repairable_indexes(client: &PostgresClient) {
    duplicate_table(client, "LiteLLM_Repairable").await;
    invalid_unique_index(client, "LiteLLM_Repairable", "LiteLLM_aaa_repairable").await;
    deduplicate(client, "LiteLLM_Repairable").await;
    duplicate_table(client, "LiteLLM_Transient").await;
    invalid_unique_index(client, "LiteLLM_Transient", "LiteLLM_ccc_leftover_ccnew1").await;
    duplicate_table(client, "Other").await;
    invalid_unique_index(client, "Other", "other_idx").await;
}

async fn create_stuck_indexes(client: &PostgresClient) {
    duplicate_table(client, "LiteLLM_Repairable").await;
    invalid_unique_index(client, "LiteLLM_Repairable", "LiteLLM_aaa_repairable").await;
    deduplicate(client, "LiteLLM_Repairable").await;
    duplicate_table(client, "LiteLLM_Stuck").await;
    invalid_unique_index(client, "LiteLLM_Stuck", "LiteLLM_zzz_stuck").await;
}

async fn assert_lock_skips_repair(
    database: &TestDatabase,
    client: &PostgresClient,
    logs: &CapturedLogs,
) {
    let holder = database.pool.get().await.unwrap();
    holder
        .execute("SELECT pg_advisory_lock($1)", &[&MIGRATION_LOCK_KEY])
        .await
        .unwrap();
    assert!(!repair_invalid_indexes(&database.check(), "30s").await);
    let captured = logs.joined();
    assert!(
        captured.contains("another process is building indexes"),
        "{captured}"
    );
    assert_still_invalid(client, &database.schema).await;
    holder
        .execute("SELECT pg_advisory_unlock($1)", &[&MIGRATION_LOCK_KEY])
        .await
        .unwrap();
}

async fn failed_repair(
    database: &TestDatabase,
    lock_timeout: &str,
    logs: &CapturedLogs,
    message: &str,
) {
    assert!(!repair_invalid_indexes(&database.check(), lock_timeout).await);
    let captured = logs.joined();
    assert!(captured.contains(message), "{captured}");
}

async fn assert_still_invalid(client: &PostgresClient, schema: &str) {
    let indexes = invalid_litellm_indexes(client, schema).await.unwrap();
    assert_eq!(
        index_names(&indexes),
        ["LiteLLM_aaa_repairable", "LiteLLM_zzz_stuck"]
    );
}

async fn assert_complete_repair(database: &TestDatabase, client: &PostgresClient) {
    assert!(repair_invalid_indexes(&database.check(), "30s").await);
    assert_eq!(
        index_valid(client, &database.schema, "LiteLLM_zzz_stuck").await,
        Some(true)
    );
    let lock: bool = client
        .query_one("SELECT pg_try_advisory_lock($1)", &[&INDEX_REPAIR_LOCK_KEY])
        .await
        .unwrap()
        .get(0);
    assert!(lock);
    client
        .execute("SELECT pg_advisory_unlock($1)", &[&INDEX_REPAIR_LOCK_KEY])
        .await
        .unwrap();
}
