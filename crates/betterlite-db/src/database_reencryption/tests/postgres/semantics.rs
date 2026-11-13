use betterlite_secrets::secret_codec::Algorithm;
use secrecy::SecretString;

use super::super::fixtures::{cipher, column, sealed, MASTER_KEY, PREVIOUS_KEY, SALT_KEY};
use super::support::{database, master, migrate, plaintexts, previous, Database};
use crate::database_reencryption::counting::count_values_encrypted_with;
use crate::database_reencryption::outcome::{MigrationOutcome, NothingToMigrate};
use crate::database_reencryption::registry::SecretColumn;
use crate::database_reencryption::storage::SecretStorage as _;

async fn count(database: &Database, key: &SecretString) -> usize {
    count_values_encrypted_with(&database.client, key, 100)
        .await
        .expect("the count succeeds")
}

async fn row_text(database: &Database, column: &SecretColumn, key: &str) -> String {
    database
        .stored(column, key)
        .await
        .expect("the row exists")
        .expect("the column is not null")
}

fn credentials() -> SecretColumn {
    column("LiteLLM_CredentialsTable", "credential_values")
}

async fn migrate_once(database: &Database, algorithm: Algorithm) -> MigrationOutcome {
    migrate(database, &previous(), &master(), &cipher(algorithm))
        .await
        .expect("the run succeeds")
        .expect("an outcome")
}

#[tokio::test]
async fn postgres_missing_columns_skipped_in_registry_order() {
    let database = database().await;
    database
        .execute("CREATE TABLE \"LiteLLM_UserTable\" (user_id text PRIMARY KEY, metadata jsonb)")
        .await;
    database
        .execute(
            "CREATE TABLE \"LiteLLM_CredentialsTable\" (credential_id text PRIMARY KEY, credential_values jsonb)",
        )
        .await;
    let found = database
        .client
        .secret_columns()
        .await
        .expect("discovery succeeds");
    assert_eq!(
        found
            .iter()
            .map(|entry| (entry.table, entry.column))
            .collect::<Vec<_>>(),
        vec![
            ("LiteLLM_CredentialsTable", "credential_values"),
            ("LiteLLM_UserTable", "metadata"),
        ]
    );
    database.close().await;
}

#[tokio::test]
async fn postgres_search_path_discovery() {
    let database = database().await;
    let current: String = database
        .client
        .query_one("SELECT current_schema()", &[])
        .await
        .expect("the current schema reads")
        .get(0);
    let hidden = format!("{current}_hidden");
    database
        .execute(&format!("CREATE SCHEMA \"{hidden}\""))
        .await;
    database
        .execute(&format!(
            "CREATE TABLE \"{hidden}\".\"LiteLLM_CredentialsTable\" (credential_id text PRIMARY KEY, credential_values jsonb)"
        ))
        .await;
    assert!(
        database
            .client
            .secret_columns()
            .await
            .expect("discovery succeeds")
            .is_empty(),
        "a schema outside the search path is not discovered"
    );
    database
        .execute(&format!("SET search_path TO \"{hidden}\""))
        .await;
    let found = database
        .client
        .secret_columns()
        .await
        .expect("discovery succeeds");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].table, "LiteLLM_CredentialsTable");
    database.close().await;
}

#[tokio::test]
async fn postgres_sql_null_vs_json_null() {
    let database = database().await;
    let column = credentials();
    database.create_table(&column).await;
    database
        .execute(
            "INSERT INTO \"LiteLLM_CredentialsTable\" (credential_id, credential_values) VALUES ('sql-null', NULL)",
        )
        .await;
    database.insert(&column, "json-null", "null").await;
    assert_eq!(count(&database, &previous()).await, 0);
    let before = database.snapshot(&column).await;
    let outcome = migrate_once(&database, Algorithm::XSalsa20Poly1305).await;
    assert!(matches!(
        outcome,
        MigrationOutcome::Nothing(NothingToMigrate::NothingLeftToMigrate)
    ));
    assert_eq!(database.snapshot(&column).await, before);
    let flags = database
        .client
        .query(
            "SELECT credential_values IS NULL FROM \"LiteLLM_CredentialsTable\" ORDER BY credential_id",
            &[],
        )
        .await
        .expect("the nulls read");
    assert!(!flags[0].get::<_, bool>(0), "the JSON null is not SQL NULL");
    assert!(flags[1].get::<_, bool>(0), "the SQL NULL stays NULL");
    database.close().await;
}

#[tokio::test]
async fn postgres_marked_metadata_prefilter() {
    let database = database().await;
    let column = column("LiteLLM_TeamTable", "metadata");
    database.create_table(&column).await;
    let marked = sealed("marked-secret", PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    let unmarked = sealed("unmarked-secret", PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    let marked_value = format!("{{\"secret\":\"litellm_enc::{marked}\"}}");
    database.insert(&column, "marked", &marked_value).await;
    let unmarked_value = format!("{{\"secret\":\"{unmarked}\"}}");
    database.insert(&column, "unmarked", &unmarked_value).await;
    assert_eq!(
        count(&database, &previous()).await,
        1,
        "only the marked row is selected"
    );
    let unmarked_before = row_text(&database, &column, "unmarked").await;
    let outcome = migrate_once(&database, Algorithm::XSalsa20Poly1305).await;
    assert!(matches!(
        outcome,
        MigrationOutcome::Migrated {
            migrated: 1,
            remaining: 0
        }
    ));
    let marked_after = row_text(&database, &column, "marked").await;
    assert_eq!(
        plaintexts(&marked_after, true, MASTER_KEY),
        vec!["marked-secret"]
    );
    assert_eq!(
        row_text(&database, &column, "unmarked").await,
        unmarked_before
    );
    database.close().await;
}

#[tokio::test]
async fn postgres_like_underscore_and_marker_in_key() {
    let database = database().await;
    let column = column("LiteLLM_TeamTable", "metadata");
    database.create_table(&column).await;
    let real = sealed("real-secret", PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    database
        .insert(&column, "key-marker", "{\"litellm_enc::key\":\"plain\"}")
        .await;
    database
        .insert(&column, "wildcard", "{\"secret\":\"litellm-enc::plain\"}")
        .await;
    database
        .insert(
            &column,
            "real",
            &format!("{{\"secret\":\"litellm_enc::{real}\"}}"),
        )
        .await;
    let key_marker_before = row_text(&database, &column, "key-marker").await;
    let wildcard_before = row_text(&database, &column, "wildcard").await;
    assert_eq!(count(&database, &previous()).await, 1);
    let outcome = migrate_once(&database, Algorithm::XSalsa20Poly1305).await;
    assert!(matches!(
        outcome,
        MigrationOutcome::Migrated {
            migrated: 1,
            remaining: 0
        }
    ));
    assert_eq!(
        row_text(&database, &column, "key-marker").await,
        key_marker_before
    );
    assert_eq!(
        row_text(&database, &column, "wildcard").await,
        wildcard_before
    );
    database.close().await;
}

#[tokio::test]
async fn postgres_selected_metadata_unmarked_leaf() {
    let database = database().await;
    let column = column("LiteLLM_UserTable", "metadata");
    database.create_table(&column).await;
    let marked = sealed("first", PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    let unmarked = sealed("second", PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    database
        .insert(
            &column,
            "user-1",
            &format!("{{\"a\":\"litellm_enc::{marked}\",\"b\":\"{unmarked}\"}}"),
        )
        .await;
    assert_eq!(count(&database, &previous()).await, 2);
    let outcome = migrate_once(&database, Algorithm::XSalsa20Poly1305).await;
    assert!(matches!(
        outcome,
        MigrationOutcome::Migrated {
            migrated: 2,
            remaining: 0
        }
    ));
    let text = row_text(&database, &column, "user-1").await;
    let mut found = plaintexts(&text, true, MASTER_KEY);
    found.sort();
    assert_eq!(found, vec!["first", "second"]);
    database.close().await;
}

#[tokio::test]
async fn postgres_leaf_count_without_writes() {
    let database = database().await;
    let column = credentials();
    database.create_table(&column).await;
    let first = sealed("one", PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    let second = sealed("two", PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    let third = sealed("three", PREVIOUS_KEY, Algorithm::Aes256Gcm);
    database
        .insert(
            &column,
            "credential-1",
            &format!("[\"{first}\",{{\"nested\":\"{second}\"}},\"{third}\",\"plain\"]"),
        )
        .await;
    let before = database.snapshot(&column).await;
    assert_eq!(count(&database, &previous()).await, 3);
    assert_eq!(database.snapshot(&column).await, before);
    database.close().await;
}

#[tokio::test]
async fn postgres_legacy_and_aes_reencrypted() {
    let database = database().await;
    let column = credentials();
    database.create_table(&column).await;
    let legacy = sealed("legacy", PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    let aes = sealed("aes", PREVIOUS_KEY, Algorithm::Aes256Gcm);
    database
        .insert(&column, "legacy", &format!("{{\"secret\":\"{legacy}\"}}"))
        .await;
    database
        .insert(&column, "aes", &format!("{{\"secret\":\"{aes}\"}}"))
        .await;
    let outcome = migrate_once(&database, Algorithm::Aes256Gcm).await;
    assert!(matches!(
        outcome,
        MigrationOutcome::Migrated {
            migrated: 2,
            remaining: 0
        }
    ));
    let migrated_legacy = row_text(&database, &column, "legacy").await;
    let migrated_aes = row_text(&database, &column, "aes").await;
    assert!(migrated_legacy.contains("v2:gcm:"), "the target codec wins");
    assert!(migrated_aes.contains("v2:gcm:"), "the target codec wins");
    assert_eq!(plaintexts(&migrated_legacy, true, MASTER_KEY).len(), 1);
    assert_eq!(plaintexts(&migrated_aes, true, MASTER_KEY).len(), 1);
    assert!(plaintexts(&migrated_legacy, true, PREVIOUS_KEY).is_empty());
    assert!(plaintexts(&migrated_aes, true, PREVIOUS_KEY).is_empty());
    database.close().await;
}

#[tokio::test]
async fn postgres_empty_plaintext_counts() {
    let database = database().await;
    let column = credentials();
    database.create_table(&column).await;
    let empty = sealed("", PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    database
        .insert(&column, "empty", &format!("{{\"secret\":\"{empty}\"}}"))
        .await;
    assert_eq!(count(&database, &previous()).await, 1);
    let outcome = migrate_once(&database, Algorithm::XSalsa20Poly1305).await;
    assert!(matches!(
        outcome,
        MigrationOutcome::Migrated {
            migrated: 1,
            remaining: 0
        }
    ));
    let text = row_text(&database, &column, "empty").await;
    assert_eq!(plaintexts(&text, true, MASTER_KEY), vec![String::new()]);
    database.close().await;
}

#[tokio::test]
async fn unknown_ciphertext_and_empty_base64_unchanged() {
    let database = database().await;
    let column = credentials();
    database.create_table(&column).await;
    let valid = sealed("valid", PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    let empty = String::from("{\"secret\":\"\"}");
    database.insert(&column, "empty", &empty).await;
    let star = String::from("{\"secret\":\"*\"}");
    database.insert(&column, "star", &star).await;
    let plain = String::from("{\"secret\":\"not a ciphertext\"}");
    database.insert(&column, "plain", &plain).await;
    let valid_value = format!("{{\"secret\":\"{valid}\"}}");
    database.insert(&column, "valid", &valid_value).await;
    let before = database.snapshot_except(&column, "valid").await;
    assert_eq!(count(&database, &previous()).await, 1);
    let outcome = migrate_once(&database, Algorithm::XSalsa20Poly1305).await;
    assert!(matches!(
        outcome,
        MigrationOutcome::Migrated {
            migrated: 1,
            remaining: 0
        }
    ));
    assert_eq!(database.snapshot_except(&column, "valid").await, before);
    database.close().await;
}

#[tokio::test]
async fn empty_target_uses_configured_key() {
    let database = database().await;
    let column = credentials();
    database.create_table(&column).await;
    let legacy = sealed("targetless", PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    database
        .insert(
            &column,
            "credential-1",
            &format!("{{\"secret\":\"{legacy}\"}}"),
        )
        .await;
    let empty_target = SecretString::from("");
    let outcome = migrate(
        &database,
        &previous(),
        &empty_target,
        &cipher(Algorithm::XSalsa20Poly1305),
    )
    .await
    .expect("the run succeeds")
    .expect("an outcome");
    assert!(matches!(
        outcome,
        MigrationOutcome::Migrated {
            migrated: 1,
            remaining: 0
        }
    ));
    let text = row_text(&database, &column, "credential-1").await;
    assert_eq!(
        plaintexts(&text, true, SALT_KEY),
        vec!["targetless"],
        "an empty target falls back to the configured signing key"
    );
    database.close().await;
}

#[tokio::test]
async fn postgres_zero_count_no_update() {
    let database = database().await;
    let column = credentials();
    database.create_table(&column).await;
    database
        .insert(&column, "credential-1", "{\"secret\":\"plain\"}")
        .await;
    let version = database.version(&column, "credential-1").await;
    let outcome = migrate_once(&database, Algorithm::XSalsa20Poly1305).await;
    assert!(matches!(
        outcome,
        MigrationOutcome::Nothing(NothingToMigrate::NothingLeftToMigrate)
    ));
    assert_eq!(
        database.version(&column, "credential-1").await,
        version,
        "no update ran"
    );
    database.close().await;
}
