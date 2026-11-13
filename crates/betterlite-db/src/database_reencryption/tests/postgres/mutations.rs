use betterlite_secrets::secret_codec::Algorithm;

use super::super::fixtures::{MASTER_KEY, PREVIOUS_KEY, cipher, column, sealed};
use super::support::{Database, database, master, migrate, plaintexts, previous, stored_value};
use crate::database_reencryption::outcome::{MigrationOutcome, NothingToMigrate};
use crate::database_reencryption::registry::SecretColumn;
use crate::database_reencryption::storage::SecretStorage as _;
use crate::database_reencryption::traversal::as_sql_parameter;

fn credentials() -> SecretColumn {
    column("LiteLLM_CredentialsTable", "credential_values")
}

fn migrated_leaves(outcome: &MigrationOutcome) -> usize {
    match outcome {
        MigrationOutcome::Migrated { migrated, .. } => *migrated,
        MigrationOutcome::Nothing(_) => 0,
        MigrationOutcome::Failed(failure) => panic!("the run failed: {}", failure.message),
    }
}

async fn run(database: &Database) -> MigrationOutcome {
    migrate(
        database,
        &previous(),
        &master(),
        &cipher(Algorithm::XSalsa20Poly1305),
    )
    .await
    .expect("the run returns an outcome")
    .expect("an outcome")
}

async fn legacy_value(database: &Database, column: &SecretColumn, key: &str) {
    let sealed = sealed("hunter2", PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    let value = if column.marked_only {
        format!("{{\"secret\":\"litellm_enc::{sealed}\"}}")
    } else {
        format!("{{\"secret\":\"{sealed}\"}}")
    };
    database.insert(column, key, &value).await;
}

async fn stored_text(database: &Database, column: &SecretColumn, key: &str) -> String {
    database
        .stored(column, key)
        .await
        .expect("the row exists")
        .expect("the column is not null")
}

#[tokio::test]
async fn postgres_jsonb_structural_cas() {
    let database = database().await;
    let column = credentials();
    database.create_table(&column).await;
    let legacy = sealed("structured", PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    database
        .execute(&format!(
            "INSERT INTO \"LiteLLM_CredentialsTable\" VALUES ('credential-1', '{{ \"b\": 2, \"a\": \"{legacy}\" }}'::jsonb)"
        ))
        .await;
    let outcome = run(&database).await;
    assert_eq!(
        migrated_leaves(&outcome),
        1,
        "the CAS compares jsonb structures"
    );
    let text = stored_text(&database, &column, "credential-1").await;
    assert_eq!(plaintexts(&text, true, MASTER_KEY), vec!["structured"]);
    database.close().await;
}

#[tokio::test]
async fn postgres_concurrent_change_preserved() {
    let database = database().await;
    let column = credentials();
    database.create_table(&column).await;
    legacy_value(&database, &column, "credential-1").await;
    let rows = database.client.rows(&column).await.expect("the rows read");
    let old = as_sql_parameter(rows[0].value.as_ref().expect("not null"), true)
        .expect("the old parameter serializes");
    database
        .execute(
            "UPDATE \"LiteLLM_CredentialsTable\" SET credential_values = '{\"edited\":true}'::jsonb WHERE credential_id = 'credential-1'",
        )
        .await;
    let updated = database
        .client
        .replace(&column, &old, "{\"secret\":\"new\"}", "credential-1")
        .await
        .expect("the CAS runs");
    assert!(!updated, "a stale row does not match");
    let text = stored_text(&database, &column, "credential-1").await;
    assert_eq!(
        serde_json::to_string(&stored_value(&text, true)).expect("the value serializes"),
        "{\"edited\":true}",
        "the concurrent edit survives"
    );
    database.close().await;
}

async fn install_rejecting_trigger(database: &Database) -> String {
    let current: String = database
        .client
        .query_one("SELECT current_schema()", &[])
        .await
        .expect("the current schema reads")
        .get(0);
    let reject = format!("{current}_reject_update");
    database
        .execute(&format!(
            "CREATE FUNCTION \"{reject}\"() RETURNS trigger AS $$ BEGIN RAISE EXCEPTION 'the user table is busy'; END $$ LANGUAGE plpgsql"
        ))
        .await;
    database
        .execute(&format!(
            "CREATE TRIGGER reject_update BEFORE UPDATE ON \"LiteLLM_UserTable\" FOR EACH ROW EXECUTE FUNCTION \"{reject}\"()"
        ))
        .await;
    reject
}

#[tokio::test]
async fn postgres_partial_progress_after_failure() {
    let database = database().await;
    let credentials_column = credentials();
    let users = column("LiteLLM_UserTable", "metadata");
    database.create_table(&credentials_column).await;
    database.create_table(&users).await;
    legacy_value(&database, &credentials_column, "credential-1").await;
    legacy_value(&database, &users, "user-1").await;
    install_rejecting_trigger(&database).await;
    let interrupted = run(&database).await;
    assert!(matches!(interrupted, MigrationOutcome::Failed(_)));
    let migrated = stored_text(&database, &credentials_column, "credential-1").await;
    assert_eq!(
        plaintexts(&migrated, true, MASTER_KEY),
        vec!["hunter2"],
        "the first column committed before the failure"
    );
    database
        .execute("DROP TRIGGER reject_update ON \"LiteLLM_UserTable\"")
        .await;
    let resumed = run(&database).await;
    assert_eq!(migrated_leaves(&resumed), 1, "the restart resumes the rest");
    let user = stored_text(&database, &users, "user-1").await;
    assert_eq!(plaintexts(&user, true, MASTER_KEY), vec!["hunter2"]);
    database.close().await;
}

#[tokio::test]
async fn postgres_two_workers_resumable() {
    let database = database().await;
    let column = credentials();
    database.create_table(&column).await;
    legacy_value(&database, &column, "credential-0").await;
    legacy_value(&database, &column, "credential-1").await;
    let previous = previous();
    let master = master();
    let cipher = cipher(Algorithm::XSalsa20Poly1305);
    let (run_one, run_two) = tokio::join!(
        migrate(&database, &previous, &master, &cipher),
        migrate(&database, &previous, &master, &cipher)
    );
    let outcome_one = run_one.expect("run one").expect("outcome one");
    let outcome_two = run_two.expect("run two").expect("outcome two");
    assert_eq!(
        migrated_leaves(&outcome_one).saturating_add(migrated_leaves(&outcome_two)),
        2,
        "each row migrates once across both workers"
    );
    let first = stored_text(&database, &column, "credential-0").await;
    assert_eq!(plaintexts(&first, true, MASTER_KEY), vec!["hunter2"]);
    let second = stored_text(&database, &column, "credential-1").await;
    assert_eq!(plaintexts(&second, true, MASTER_KEY), vec!["hunter2"]);
    let third = run(&database).await;
    assert!(matches!(
        third,
        MigrationOutcome::Nothing(NothingToMigrate::NothingLeftToMigrate)
    ));
    database.close().await;
}

#[tokio::test]
async fn malformed_row_or_primary_key_fails() {
    let database = database().await;
    database
        .execute(
            "CREATE TABLE \"LiteLLM_CredentialsTable\" (credential_id integer PRIMARY KEY, credential_values jsonb)",
        )
        .await;
    database
        .execute(
            "INSERT INTO \"LiteLLM_CredentialsTable\" VALUES (1, '{\"secret\":\"plain\"}'::jsonb)",
        )
        .await;
    let integer_key = run(&database).await;
    let MigrationOutcome::Failed(key_failure) = integer_key else {
        panic!("a non-string primary key fails")
    };
    assert_eq!(key_failure.class, "InvalidStoredRow");
    database
        .execute("DROP TABLE \"LiteLLM_CredentialsTable\"")
        .await;
    database
        .execute(
            "CREATE TABLE \"LiteLLM_CredentialsTable\" (credential_id text PRIMARY KEY, credential_values text)",
        )
        .await;
    database
        .execute("INSERT INTO \"LiteLLM_CredentialsTable\" VALUES ('bad', 'not json')")
        .await;
    let malformed = run(&database).await;
    let MigrationOutcome::Failed(decode_failure) = malformed else {
        panic!("a malformed stored value fails")
    };
    assert_eq!(decode_failure.class, "StoredValueDecodeError");
    database.close().await;
}
