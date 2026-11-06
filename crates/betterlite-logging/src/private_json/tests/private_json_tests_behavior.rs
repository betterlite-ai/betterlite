use std::fs;
use std::io;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;
use serde::Serialize;
use serde_json::Value;
use tempfile::TempDir;
use tokio::time::{sleep, timeout};
use tokio::{fs as async_fs, task};

use super::super::error::PrivateJsonError;
use super::super::files::{
    FileAccess, LocalFiles, MAX_CONCURRENT_FILE_OPERATIONS, PrivateDocument, PrivateFiles,
    StagedFile,
};
use super::super::safe_json::{
    DEFAULT_MAX_RECURSE_DEPTH, MAX_JSON_BYTES, MAX_JSON_DEPTH, Passthrough, ValueTransform,
    safe_dumps, safe_json_loads,
};
use super::{
    Fault, Gate, ScriptedFiles, cancel_gated_calls, entries_of, mode_of, value, wait_until_entered,
};

struct KeyPrefix;

impl ValueTransform for KeyPrefix {
    fn transform(&self, key: Option<&str>, value: &str) -> String {
        format!("{}:{value}", key.unwrap_or("root"))
    }
}

#[derive(Serialize)]
struct CacheRecord {
    key: String,
    payload: Passthrough,
}

fn private_files_with(fault: Fault) -> PrivateFiles {
    let access: Arc<dyn FileAccess> = Arc::new(ScriptedFiles { fault, gate: None });
    PrivateFiles::from(access)
}

fn document(text: &str) -> PrivateDocument {
    PrivateDocument::json(&value(text)).expect("encodes the document")
}

#[rstest]
#[case::value_at_the_bound_survives(1, r#"{"a": "kept"}"#, r#"{"a": "kept"}"#)]
#[case::value_past_the_bound_becomes_the_marker(
    0,
    r#"{"a": "kept"}"#,
    r#"{"a": "MaxDepthExceeded"}"#
)]
#[case::marker_replaces_items_past_the_bound(
    1,
    r#"{"a": ["x"]}"#,
    r#"{"a": ["MaxDepthExceeded"]}"#
)]
fn safe_dumps_replaces_values_past_max_depth_with_the_marker(
    #[case] max_depth: usize,
    #[case] data: &str,
    #[case] expected: &str,
) {
    let dumped = safe_dumps(&value(data), max_depth, None).expect("dumps the data");
    assert_eq!(value(&dumped), value(expected));
}

#[test]
fn safe_dumps_passes_the_enclosing_key_to_the_transform() {
    let data = value(r#"{"token": ["a", {"nested": "b"}], "plain": "c"}"#);
    let transform: &dyn ValueTransform = &KeyPrefix;
    let dumped =
        safe_dumps(&data, DEFAULT_MAX_RECURSE_DEPTH, Some(transform)).expect("dumps the data");
    assert_eq!(
        value(&dumped),
        value(r#"{"token": ["token:a", {"nested": "nested:b"}], "plain": "plain:c"}"#)
    );
}

#[test]
fn safe_dumps_redacts_credentials_with_the_b187_redactor() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("builds the redactor");
    let data = value(r#"{"api_key": "hunter2hunter2", "model": "gpt-4o"}"#);
    let transform: &dyn ValueTransform = &redactor;
    let dumped =
        safe_dumps(&data, DEFAULT_MAX_RECURSE_DEPTH, Some(transform)).expect("dumps the data");
    assert_eq!(
        value(&dumped),
        value(r#"{"api_key": "REDACTED", "model": "gpt-4o"}"#)
    );
}

#[test]
fn safe_dumps_accepts_typed_records_and_the_passthrough_field() {
    let record = CacheRecord {
        key: "team".to_owned(),
        payload: Passthrough::try_from(value(r#"{"nested": [1, 2]}"#))
            .expect("bounded passthrough"),
    };
    let dumped = safe_dumps(&record, DEFAULT_MAX_RECURSE_DEPTH, None).expect("dumps the record");
    assert_eq!(
        value(&dumped),
        value(r#"{"key": "team", "payload": {"nested": [1, 2]}}"#)
    );
}

#[test]
fn passthrough_rejects_an_oversized_value() {
    let oversized = Value::String("x".repeat(MAX_JSON_BYTES));
    let error = Passthrough::try_from(oversized).expect_err("rejects oversized JSON");
    assert!(matches!(error, PrivateJsonError::TooLarge(..)));
}

#[test]
fn passthrough_rejects_a_value_nested_past_the_depth_limit() {
    let deep = (0..=MAX_JSON_DEPTH).fold(Value::Null, |inner, _| Value::Array(vec![inner]));
    let error = Passthrough::try_from(deep).expect_err("rejects deep JSON");
    assert!(matches!(error, PrivateJsonError::TooDeep(limit) if limit == MAX_JSON_DEPTH));
}

#[test]
fn safe_json_loads_keeps_the_serde_error_as_the_source() {
    let error = safe_json_loads(b"{\"model\": ").expect_err("rejects truncated json");
    let PrivateJsonError::Decode(source) = &error else {
        panic!("expected a decode error, got {error:?}");
    };
    assert!(source.is_eof());
}

#[test]
fn safe_json_loads_rejects_a_payload_over_the_byte_limit() {
    let oversized = vec![b' '; MAX_JSON_BYTES + 1];
    let error = safe_json_loads(&oversized).expect_err("rejects an oversized payload");
    assert!(matches!(
        error,
        PrivateJsonError::TooLarge(size, limit)
            if size == MAX_JSON_BYTES + 1 && limit == MAX_JSON_BYTES
    ));
}

#[rstest]
#[case::at_the_depth_limit(MAX_JSON_DEPTH, false)]
#[case::past_the_depth_limit(MAX_JSON_DEPTH + 1, true)]
fn safe_json_loads_bounds_the_nesting_depth(#[case] depth: usize, #[case] rejected: bool) {
    let text = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
    let error = safe_json_loads(text.as_bytes()).err();
    assert_eq!(error.is_some(), rejected);
    if rejected {
        assert!(matches!(error, Some(PrivateJsonError::TooDeep(limit)) if limit == MAX_JSON_DEPTH));
    }
}

#[test]
fn local_staging_is_owner_only_before_any_bytes_are_written() {
    let directory = TempDir::new().expect("temp dir");
    let staged = LocalFiles
        .create_staged(directory.path(), Some(".json"))
        .expect("creates the stage");
    assert_eq!(mode_of(&staged.path), 0o600);
    assert_eq!(fs::metadata(&staged.path).expect("metadata").len(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cancelled_caller_keeps_its_permit_until_the_blocking_job_ends() {
    let gate = Arc::new(Gate::default());
    let access: Arc<dyn FileAccess> = Arc::new(ScriptedFiles::gated(Arc::clone(&gate)));
    let files = PrivateFiles::from(access);
    let directory = TempDir::new().expect("temp dir");
    cancel_gated_calls(&files, &directory.path().join("private")).await;
    wait_until_entered(&gate, MAX_CONCURRENT_FILE_OPERATIONS).await;
    assert_eq!(gate.entered(), MAX_CONCURRENT_FILE_OPERATIONS);
    let ninth_path = directory.path().join("ninth");
    let ninth = files.ensure_private_dir(&ninth_path);
    let waited = timeout(Duration::from_millis(100), ninth).await;
    assert!(
        waited.is_err(),
        "a cancelled caller's job still holds its permit"
    );
    assert_eq!(gate.entered(), MAX_CONCURRENT_FILE_OPERATIONS);
    gate.release();
    sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn bytes_are_staged_and_written_with_owner_only_permissions() {
    let directory = TempDir::new().expect("temp dir");
    let target = directory.path().join("token");
    let files = PrivateFiles::default();
    let staged = files
        .stage_private(
            &target,
            PrivateDocument {
                suffix: None,
                bytes: b"first".to_vec(),
            },
        )
        .await
        .expect("stages the bytes");
    assert_eq!(mode_of(&staged.path), 0o600);
    assert!(!target.exists());
    files
        .write_private(
            &target,
            PrivateDocument {
                suffix: None,
                bytes: b"second".to_vec(),
            },
        )
        .await
        .expect("writes the bytes");
    assert_eq!(fs::read(&target).expect("reads"), b"second");
    assert_eq!(mode_of(&target), 0o600);
}

#[tokio::test]
async fn failed_staging_preserves_originals_and_removes_temporaries() {
    let directory = TempDir::new().expect("temp dir");
    let target = directory.path().join("cache.json");
    fs::write(&target, "{\"old\": true}").expect("writes the original");
    let files = private_files_with(Fault::Write);
    let error = files
        .stage_private(&target, document(r#"{"fresh": 1}"#))
        .await
        .expect_err("staging fails");
    assert!(matches!(error, PrivateJsonError::Stage(..)));
    assert_eq!(
        fs::read_to_string(&target).expect("reads"),
        "{\"old\": true}"
    );
    assert_eq!(entries_of(directory.path()), vec!["cache.json".to_owned()]);
}

#[tokio::test]
async fn a_failed_cleanup_keeps_the_staging_error() {
    let directory = TempDir::new().expect("temp dir");
    let target = directory.path().join("cache.json");
    let files = private_files_with(Fault::WriteAndRemove);
    let error = files
        .stage_private(&target, document(r#"{"fresh": 1}"#))
        .await
        .expect_err("staging fails");
    assert!(matches!(error, PrivateJsonError::Stage(..)));
}

#[tokio::test]
async fn a_failed_commit_keeps_the_original_and_removes_the_staged_file() {
    let directory = TempDir::new().expect("temp dir");
    let target = directory.path().join("cache.json");
    fs::write(&target, "{\"old\": true}").expect("writes the original");
    let files = private_files_with(Fault::Replace);
    let staged = files
        .stage_private(&target, document(r#"{"fresh": 1}"#))
        .await
        .expect("stages the file");
    let error = files
        .commit_staged_json(&staged, &target)
        .await
        .expect_err("commit fails");
    assert!(matches!(error, PrivateJsonError::Commit(..)));
    assert_eq!(
        fs::read_to_string(&target).expect("reads"),
        "{\"old\": true}"
    );
    assert!(!staged.path.exists());
    assert_eq!(entries_of(directory.path()), vec!["cache.json".to_owned()]);
}

#[tokio::test]
async fn discarding_a_staged_file_removes_it_and_ignores_a_missing_one() {
    let directory = TempDir::new().expect("temp dir");
    let files = PrivateFiles::default();
    let staged = files
        .stage_private(
            &directory.path().join("cache.json"),
            document(r#"{"fresh": 1}"#),
        )
        .await
        .expect("stages the file");
    files
        .discard_staged_json(&staged)
        .await
        .expect("discards the file");
    assert!(!staged.path.exists());
    files
        .discard_staged_json(&StagedFile {
            path: directory.path().join("gone.json"),
        })
        .await
        .expect("a missing stage is already discarded");
    assert!(entries_of(directory.path()).is_empty());
}

#[tokio::test]
async fn ensure_private_dir_refuses_a_path_with_no_final_name() {
    let error = PrivateFiles::default()
        .ensure_private_dir(Path::new(""))
        .await
        .expect_err("refuses an empty path");
    assert!(matches!(error, PrivateJsonError::InvalidPath(_)));
}

#[tokio::test]
async fn staging_follows_a_symlinked_parent_directory() {
    let directory = TempDir::new().expect("temp dir");
    let real = directory.path().join("real");
    fs::create_dir_all(&real).expect("creates the directory");
    let link = directory.path().join("link");
    symlink(&real, &link).expect("links the directory");
    PrivateFiles::default()
        .write_private(&link.join("state.json"), document(r#"{"through": "link"}"#))
        .await
        .expect("writes through the link");
    let written = fs::read_to_string(real.join("state.json")).expect("reads through the link");
    assert_eq!(value(&written), value(r#"{"through": "link"}"#));
    assert!(
        fs::symlink_metadata(&link)
            .expect("link metadata")
            .file_type()
            .is_symlink()
    );
}

#[tokio::test]
async fn explicit_overwrite_rejects_a_symlinked_final_target() {
    let directory = TempDir::new().expect("temp dir");
    let secret = directory.path().join("secret.json");
    fs::write(&secret, "{\"original\": true}").expect("writes the original");
    let link = directory.path().join("state.json");
    symlink(&secret, &link).expect("links the file");
    let error = PrivateFiles::default()
        .overwrite_private_json(&link, &value(r#"{"replaced": true}"#))
        .await
        .expect_err("refuses the symlink");
    assert!(matches!(error, PrivateJsonError::Overwrite(..)));
    assert_eq!(
        fs::read_to_string(&secret).expect("reads"),
        "{\"original\": true}"
    );
}

#[tokio::test]
async fn explicit_overwrite_never_creates_a_missing_file() {
    let directory = TempDir::new().expect("temp dir");
    let target = directory.path().join("missing.json");
    let error = PrivateFiles::default()
        .overwrite_private_json(&target, &value(r#"{"new": true}"#))
        .await
        .expect_err("refuses to create");
    assert!(matches!(error, PrivateJsonError::Overwrite(..)));
    assert!(!target.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_writes_never_reveal_a_partial_document() {
    let directory = TempDir::new().expect("temp dir");
    let target = directory.path().join("state.json");
    let files = PrivateFiles::default();
    let initial = value(r#"{"writer": "initial"}"#);
    files
        .write_private(&target, document(r#"{"writer": "initial"}"#))
        .await
        .expect("writes the original");
    let first = value(&format!(
        r#"{{"writer": "first", "padding": "{}"}}"#,
        "a".repeat(4096)
    ));
    let second = value(&format!(
        r#"{{"writer": "second", "padding": "{}"}}"#,
        "b".repeat(4096)
    ));
    let writer_one = files.write_private(&target, PrivateDocument::json(&first).expect("encodes"));
    let writer_two = files.write_private(&target, PrivateDocument::json(&second).expect("encodes"));
    let reader = observe_documents(&target);
    let (one, two, observed) = tokio::join!(writer_one, writer_two, reader);
    one.expect("first writer");
    two.expect("second writer");
    assert!(!observed.is_empty());
    let partial = observed
        .iter()
        .find(|document| **document != first && **document != second && **document != initial);
    assert!(
        partial.is_none(),
        "reader saw a partial document: {partial:?}"
    );
    assert_eq!(entries_of(directory.path()), vec!["state.json".to_owned()]);
}

async fn observe_documents(path: &Path) -> Vec<Value> {
    let mut observed = Vec::new();
    for _ in 0_usize..128_usize {
        match async_fs::read(path).await {
            Ok(bytes) => observed
                .push(serde_json::from_slice(&bytes).expect("a reader sees a whole document")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("read failed: {error}"),
        }
        task::yield_now().await;
    }
    observed
}
