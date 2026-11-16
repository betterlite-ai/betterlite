#![cfg(test)]

mod fakes;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Result as IoResult, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::Level;
use betterlite_secrets::secret_codec::{DecryptPolicy, FailureLog, FailureValue, StoredValue};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tracing::subscriber::set_default;
use tracing_subscriber::fmt::MakeWriter;

use super::codec::{USER_ENV_VARS_KEY, decode_user_env_vars, parse_user_env_vars};
use super::error::UserEnvVarsError;
use super::lock::user_env_vars_lock_key;
use super::store::UserEnvVars;

use fakes::{
    FakeRow, FakeTable, SALT, TestCipher, lock, recording_cipher, sealed, test_cipher, test_store,
    test_store_with, unkeyed_cipher,
};

#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    fn contents(&self) -> String {
        let bytes = lock(&self.0).clone();
        String::from_utf8(bytes).expect("the log writer records UTF-8")
    }

    fn warnings(&self) -> Vec<String> {
        self.contents()
            .lines()
            .filter(|line| line.contains("WARN"))
            .map(str::to_owned)
            .collect()
    }
}

impl Write for CapturedLogs {
    fn write(&mut self, buffer: &[u8]) -> IoResult<usize> {
        lock(&self.0).extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

fn capture_logs() -> (CapturedLogs, impl tracing::Subscriber + Send + Sync) {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(logs.clone())
        .finish();
    (logs, subscriber)
}

fn exposed(values: &UserEnvVars) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

fn texts<const N: usize>(entries: [(&str, &str); N]) -> BTreeMap<String, String> {
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}

fn names(entries: &[&str]) -> BTreeSet<String> {
    entries.iter().map(|name| name.to_string()).collect()
}

// reads a sealed column straight back, to prove what the merge wrote rather than what it returns
fn opened(cipher: &TestCipher, stored: &str) -> String {
    let value = StoredValue::<()>::Text(SecretString::from(stored.to_owned()));
    let policy = DecryptPolicy {
        log: FailureLog::Debug,
        value: FailureValue::Nothing,
    };
    match cipher
        .decrypt_value(value, USER_ENV_VARS_KEY, policy)
        .expect("the stored blob decrypts under the test key")
    {
        StoredValue::Text(text) => text.expose_secret().to_owned(),
        StoredValue::Other(()) => panic!("a text blob decrypts to text"),
    }
}

#[test]
fn decode_object_of_strings_gives_the_map() {
    let cipher = test_cipher(SALT);
    let stored = sealed(
        &cipher,
        "{\"API_KEY\":\"s3cr3t-value\",\"TOKEN\":\"tok-2\"}",
    );
    let decoded = decode_user_env_vars(Some(&stored), &cipher);
    assert_eq!(
        exposed(&decoded),
        texts([("API_KEY", "s3cr3t-value"), ("TOKEN", "tok-2")])
    );
}

#[test]
fn decode_reports_the_field_and_debug_level_when_decryption_fails() {
    let (cipher, diagnostics) = recording_cipher(SALT);
    let stored = sealed(
        &test_cipher("rotated-salt"),
        "{\"API_KEY\":\"s3cr3t-value\"}",
    );
    assert!(decode_user_env_vars(Some(&stored), &cipher).is_empty());
    let events = diagnostics.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, Level::Debug);
    assert!(events[0].1.contains("mcp_user_env_vars"));
}

#[test]
fn decode_undecryptable_blob_warns_once_and_reads_as_empty() {
    let (logs, subscriber) = capture_logs();
    let _guard = set_default(subscriber);
    let cipher = test_cipher(SALT);
    let stored = sealed(
        &test_cipher("rotated-salt"),
        "{\"API_KEY\":\"s3cr3t-value\"}",
    );
    let decoded = decode_user_env_vars(Some(&stored), &cipher);
    assert!(decoded.is_empty());
    let warnings = logs.warnings();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("LITELLM_SALT_KEY"));
    assert!(!logs.contents().contains(&stored));
}

#[test]
fn decode_without_a_signing_key_warns_and_reads_as_empty() {
    let (logs, subscriber) = capture_logs();
    let _guard = set_default(subscriber);
    let stored = sealed(&test_cipher(SALT), "{\"API_KEY\":\"s3cr3t-value\"}");
    assert!(decode_user_env_vars(Some(&stored), &unkeyed_cipher()).is_empty());
    assert_eq!(logs.warnings().len(), 1);
}

#[test]
fn decode_empty_blob_is_empty_without_a_warning() {
    let (logs, subscriber) = capture_logs();
    let _guard = set_default(subscriber);
    let decoded = decode_user_env_vars(Some(""), &test_cipher(SALT));
    assert!(decoded.is_empty());
    assert!(logs.warnings().is_empty());
}

#[test]
fn decode_missing_stored_text_is_empty() {
    assert!(decode_user_env_vars(None, &test_cipher(SALT)).is_empty());
}

#[test]
fn decode_invalid_json_is_empty_without_a_warning() {
    let (logs, subscriber) = capture_logs();
    let _guard = set_default(subscriber);
    let cipher = test_cipher(SALT);
    let stored = sealed(&cipher, "not json");
    assert!(decode_user_env_vars(Some(&stored), &cipher).is_empty());
    assert!(logs.warnings().is_empty());
}

#[rstest]
#[case::array("[1, 2]")]
#[case::text("\"text\"")]
#[case::null("null")]
#[case::number("7")]
#[case::boolean("true")]
fn decode_json_that_is_not_an_object_is_empty_without_a_warning(#[case] plaintext: &str) {
    let (logs, subscriber) = capture_logs();
    let _guard = set_default(subscriber);
    let cipher = test_cipher(SALT);
    let stored = sealed(&cipher, plaintext);
    assert!(decode_user_env_vars(Some(&stored), &cipher).is_empty());
    assert!(logs.warnings().is_empty());
}

#[test]
fn decode_non_string_value_warns_without_names_or_values() {
    let (logs, subscriber) = capture_logs();
    let _guard = set_default(subscriber);
    let cipher = test_cipher(SALT);
    let stored = sealed(&cipher, "{\"API_KEY\":\"hunter2\",\"RETRIES\":3}");
    assert!(decode_user_env_vars(Some(&stored), &cipher).is_empty());
    let warnings = logs.warnings();
    assert_eq!(warnings.len(), 1);
    assert!(!warnings[0].contains("API_KEY"));
    assert!(!warnings[0].contains("hunter2"));
    assert!(!warnings[0].contains("RETRIES"));
}

#[test]
fn decode_keeps_the_last_duplicate_name() {
    let cipher = test_cipher(SALT);
    let stored = sealed(&cipher, "{\"API_KEY\":\"old\",\"API_KEY\":\"new\"}");
    let decoded = decode_user_env_vars(Some(&stored), &cipher);
    assert_eq!(exposed(&decoded), texts([("API_KEY", "new")]));
}

#[test]
fn decode_plain_base64_is_not_a_legacy_path() {
    let (logs, subscriber) = capture_logs();
    let _guard = set_default(subscriber);
    // base64url of {"API_KEY":"legacy"}; per-user env vars have no plain-base64 fallback
    let decoded = decode_user_env_vars(Some("eyJBUElfS0VZIjoibGVnYWN5In0="), &test_cipher(SALT));
    assert!(decoded.is_empty());
    assert_eq!(logs.warnings().len(), 1);
}

#[tokio::test]
async fn get_returns_the_decoded_map_for_the_composite_key() {
    let cipher = test_cipher(SALT);
    let table = FakeTable::new(vec![
        FakeRow::new(
            "alice",
            "srv-1",
            sealed(&cipher, "{\"API_KEY\":\"s3cr3t-value\"}"),
        ),
        FakeRow::new("bob", "srv-1", sealed(&cipher, "{\"API_KEY\":\"other\"}")),
    ]);
    let store = test_store(&table);
    let values = store
        .get_user_env_vars("alice", "srv-1")
        .await
        .expect("the read succeeds");
    assert_eq!(exposed(&values), texts([("API_KEY", "s3cr3t-value")]));
    assert_eq!(table.events(), ["find_unique"]);
}

#[tokio::test]
async fn get_missing_row_is_an_empty_map() {
    let cipher = test_cipher(SALT);
    let table = FakeTable::new(vec![FakeRow::new("bob", "srv-1", sealed(&cipher, "{}"))]);
    let store = test_store(&table);
    let values = store
        .get_user_env_vars("alice", "srv-1")
        .await
        .expect("the read succeeds");
    assert!(values.is_empty());
}

#[tokio::test]
async fn get_undecryptable_row_is_an_empty_map() {
    let table = FakeTable::new(vec![FakeRow::new(
        "alice",
        "srv-1",
        sealed(
            &test_cipher("rotated-salt"),
            "{\"API_KEY\":\"s3cr3t-value\"}",
        ),
    )]);
    let store = test_store(&table);
    let values = store
        .get_user_env_vars("alice", "srv-1")
        .await
        .expect("the read succeeds");
    assert!(values.is_empty());
}

#[tokio::test]
async fn bulk_empty_server_ids_makes_no_database_call() {
    let cipher = test_cipher(SALT);
    let table = FakeTable::new(vec![FakeRow::new("alice", "srv-1", sealed(&cipher, "{}"))]);
    let store = test_store(&table);
    let found = store
        .get_user_env_vars_bulk("alice", &[])
        .await
        .expect("the bulk read succeeds");
    assert!(found.is_empty());
    assert!(table.events().is_empty());
}

#[tokio::test]
async fn bulk_returns_found_rows_omits_missing_and_keeps_unreadable() {
    let cipher = test_cipher(SALT);
    let table = FakeTable::new(vec![
        FakeRow::new(
            "alice",
            "srv-1",
            sealed(&cipher, "{\"API_KEY\":\"s3cr3t-value\"}"),
        ),
        FakeRow::new(
            "alice",
            "srv-2",
            sealed(&test_cipher("rotated-salt"), "{\"API_KEY\":\"old\"}"),
        ),
        FakeRow::new("bob", "srv-3", sealed(&cipher, "{\"API_KEY\":\"other\"}")),
    ]);
    let store = test_store(&table);
    let ids = ["srv-1".to_owned(), "srv-2".to_owned(), "srv-3".to_owned()];
    let found = store
        .get_user_env_vars_bulk("alice", &ids)
        .await
        .expect("the bulk read succeeds");
    assert_eq!(found.len(), 2);
    assert_eq!(
        found.get("srv-1").map(exposed),
        Some(texts([("API_KEY", "s3cr3t-value")]))
    );
    assert!(found.get("srv-2").is_some_and(BTreeMap::is_empty));
    assert!(!found.contains_key("srv-3"));
    assert_eq!(table.events(), ["find_many"]);
}

#[tokio::test]
async fn bulk_propagates_a_database_error() {
    let table = FakeTable::failing(Vec::new(), "find_many");
    let store = test_store(&table);
    let result = store
        .get_user_env_vars_bulk("alice", &["srv-1".to_owned()])
        .await;
    assert!(matches!(result, Err(UserEnvVarsError::Database(_))));
}

#[test]
fn lock_key_matches_lite_llm_blake2b_over_the_bare_text() {
    assert_eq!(
        user_env_vars_lock_key("alice", "srv-1"),
        3_488_191_928_229_801_122
    );
    assert_eq!(
        user_env_vars_lock_key("user-1", "server-1"),
        -8_965_162_404_798_766_016
    );
}

#[tokio::test]
async fn lock_runs_before_the_row_read_inside_the_transaction() {
    let cipher = test_cipher(SALT);
    let table = FakeTable::new(vec![FakeRow::new(
        "alice",
        "srv-1",
        sealed(&cipher, "{\"KEPT\":\"old\"}"),
    )]);
    let store = test_store(&table);
    drop(
        store
            .merge_user_env_vars(
                "alice",
                "srv-1",
                &texts([("FRESH", "v")]),
                &names(&["FRESH", "KEPT"]),
            )
            .await
            .expect("the merge commits"),
    );
    assert_eq!(table.events(), ["tx", "lock", "find_unique", "upsert"]);
    assert_eq!(table.locks(), [user_env_vars_lock_key("alice", "srv-1")]);
}

#[tokio::test]
async fn lock_failure_stops_before_the_read() {
    let table = FakeTable::failing(Vec::new(), "lock");
    let store = test_store(&table);
    let result = store
        .merge_user_env_vars(
            "alice",
            "srv-1",
            &texts([("API_KEY", "v")]),
            &names(&["API_KEY"]),
        )
        .await;
    assert!(matches!(result, Err(UserEnvVarsError::Database(_))));
    assert_eq!(table.events(), ["tx", "lock"]);
}

#[tokio::test]
async fn merge_creates_the_row_when_absent() {
    let cipher = test_cipher(SALT);
    let table = FakeTable::new(Vec::new());
    let store = test_store(&table);
    let merged = store
        .merge_user_env_vars(
            "alice",
            "srv-1",
            &texts([("API_KEY", "fresh")]),
            &names(&["API_KEY"]),
        )
        .await
        .expect("the merge commits");
    assert_eq!(exposed(&merged), texts([("API_KEY", "fresh")]));
    let rows = table.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].user_id, "alice");
    assert_eq!(rows[0].server_id, "srv-1");
    assert_eq!(
        opened(&cipher, &rows[0].values_b64),
        "{\"API_KEY\":\"fresh\"}"
    );
}

#[tokio::test]
async fn merge_overlays_updates_and_prunes_retired_names() {
    let cipher = test_cipher(SALT);
    let table = FakeTable::new(vec![FakeRow::new(
        "alice",
        "srv-1",
        sealed(&cipher, "{\"KEPT\":\"old\",\"RETIRED\":\"gone\"}"),
    )]);
    let store = test_store(&table);
    let merged = store
        .merge_user_env_vars(
            "alice",
            "srv-1",
            &texts([("FRESH", "v"), ("KEPT", "new"), ("UNLISTED", "x")]),
            &names(&["FRESH", "KEPT"]),
        )
        .await
        .expect("the merge commits");
    assert_eq!(exposed(&merged), texts([("FRESH", "v"), ("KEPT", "new")]));
    assert_eq!(
        opened(&cipher, &table.rows()[0].values_b64),
        "{\"FRESH\":\"v\",\"KEPT\":\"new\"}"
    );
}

#[tokio::test]
async fn merge_writes_a_sealed_empty_object_for_an_empty_result() {
    let cipher = test_cipher(SALT);
    let table = FakeTable::new(vec![FakeRow::new(
        "alice",
        "srv-1",
        sealed(&cipher, "{\"RETIRED\":\"gone\"}"),
    )]);
    let store = test_store(&table);
    let merged = store
        .merge_user_env_vars("alice", "srv-1", &BTreeMap::new(), &BTreeSet::new())
        .await
        .expect("the merge commits");
    assert!(merged.is_empty());
    let rows = table.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(opened(&cipher, &rows[0].values_b64), "{}");
}

#[tokio::test]
async fn merge_replaces_an_undecryptable_row() {
    let cipher = test_cipher(SALT);
    let table = FakeTable::new(vec![FakeRow::new(
        "alice",
        "srv-1",
        sealed(&test_cipher("rotated-salt"), "{\"OLD\":\"value\"}"),
    )]);
    let store = test_store(&table);
    let merged = store
        .merge_user_env_vars(
            "alice",
            "srv-1",
            &texts([("API_KEY", "fresh")]),
            &names(&["API_KEY"]),
        )
        .await
        .expect("the merge commits");
    assert_eq!(exposed(&merged), texts([("API_KEY", "fresh")]));
    assert_eq!(
        opened(&cipher, &table.rows()[0].values_b64),
        "{\"API_KEY\":\"fresh\"}"
    );
}

#[tokio::test]
async fn merge_propagates_a_database_error_without_logging_a_value() {
    let (logs, subscriber) = capture_logs();
    let _guard = set_default(subscriber);
    let cipher = test_cipher(SALT);
    let table = FakeTable::failing(
        vec![FakeRow::new(
            "alice",
            "srv-1",
            sealed(&cipher, "{\"API_KEY\":\"kept\"}"),
        )],
        "upsert",
    );
    let store = test_store(&table);
    let result = store
        .merge_user_env_vars(
            "alice",
            "srv-1",
            &texts([("API_KEY", "hunter2")]),
            &names(&["API_KEY"]),
        )
        .await;
    assert!(matches!(result, Err(UserEnvVarsError::Database(_))));
    assert_eq!(
        opened(&cipher, &table.rows()[0].values_b64),
        "{\"API_KEY\":\"kept\"}"
    );
    assert!(!logs.contents().contains("hunter2"));
}

#[tokio::test]
async fn merge_propagates_a_cipher_error_without_logging_a_value() {
    let (logs, subscriber) = capture_logs();
    let _guard = set_default(subscriber);
    let table = FakeTable::new(Vec::new());
    let store = test_store_with(&table, unkeyed_cipher());
    let result = store
        .merge_user_env_vars(
            "alice",
            "srv-1",
            &texts([("API_KEY", "hunter2")]),
            &names(&["API_KEY"]),
        )
        .await;
    assert!(matches!(
        result,
        Err(UserEnvVarsError::Encrypt(SecretError::MissingKey))
    ));
    assert!(table.rows().is_empty());
    assert!(!logs.contents().contains("hunter2"));
}

#[tokio::test]
async fn delete_removes_the_row_with_one_delete_many() {
    let cipher = test_cipher(SALT);
    let table = FakeTable::new(vec![
        FakeRow::new("alice", "srv-1", sealed(&cipher, "{}")),
        FakeRow::new("alice", "srv-2", sealed(&cipher, "{}")),
    ]);
    let store = test_store(&table);
    store
        .delete_user_env_vars("alice", "srv-1")
        .await
        .expect("the delete succeeds");
    assert_eq!(table.events(), ["delete_many"]);
    let rows = table.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].server_id, "srv-2");
}

#[tokio::test]
async fn delete_missing_row_is_a_no_op() {
    let table = FakeTable::new(Vec::new());
    let store = test_store(&table);
    store
        .delete_user_env_vars("alice", "srv-1")
        .await
        .expect("the delete succeeds");
    assert!(table.rows().is_empty());
}

#[tokio::test]
async fn delete_propagates_a_database_error() {
    let table = FakeTable::failing(Vec::new(), "delete_many");
    let store = test_store(&table);
    let result = store.delete_user_env_vars("alice", "srv-1").await;
    assert!(matches!(result, Err(UserEnvVarsError::Database(_))));
}

#[rstest]
fn replays_the_mcp_user_env_vars_corpus(#[files("fuzz/corpus/mcp_user_env_vars/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("the corpus entry reads");
    if let Ok(text) = std::str::from_utf8(&bytes) {
        drop(parse_user_env_vars(text));
    }
}
