#![cfg(test)]

use betterlite_db::mcp_stored_contracts::JsonValue;

use super::super::env_vars::reencrypt_global_env_values;
use super::super::error::RotationCodecError;
use super::super::rotation::Rotation;
use super::super::store::StoredServerRow;
use super::support::{
    FakeStore, NEW_KEY, OLD_KEY, capture_logs, cipher, empty_store, json, new_key, object, sealed,
    string,
};

fn stored(value: &JsonValue) -> Option<String> {
    Some(serde_json::to_string(value).unwrap())
}

fn env_entry(name: &str, value: JsonValue) -> JsonValue {
    object(&[("name", string(name)), ("value", value)])
}

fn scoped_entry(name: &str, value: JsonValue, scope: JsonValue) -> JsonValue {
    object(&[("name", string(name)), ("value", value), ("scope", scope)])
}

fn array(entries: Vec<JsonValue>) -> JsonValue {
    JsonValue::Array(entries)
}

fn rotated_entries(result: Option<JsonValue>) -> Vec<JsonValue> {
    let Some(JsonValue::String(text)) = result else {
        panic!("a rotated env var column is a JSON text string");
    };
    let JsonValue::Array(entries) = json(&text) else {
        panic!("the rotated env var text is a JSON array");
    };
    entries
}

fn only_server(env_vars: Option<String>) -> FakeStore {
    let mut store = empty_store();
    store.servers = vec![StoredServerRow {
        server_id: "s1".to_owned(),
        credentials: None,
        env_vars,
        static_headers: None,
        env: None,
    }];
    store
}

fn scope_entries() -> JsonValue {
    array(vec![
        scoped_entry(
            "user-var",
            string(&sealed("user-value", OLD_KEY)),
            string("user"),
        ),
        env_entry("missing-scope", string(&sealed("missing-value", OLD_KEY))),
        scoped_entry(
            "global-scope",
            string(&sealed("global-value", OLD_KEY)),
            string("global"),
        ),
        scoped_entry(
            "odd-scope",
            string(&sealed("odd-value", OLD_KEY)),
            json("7"),
        ),
        env_entry("null-value", JsonValue::Null),
        env_entry("empty-value", string("")),
        env_entry("zero-value", json("0")),
        env_entry("false-value", JsonValue::Bool(false)),
        env_entry("list-value", json("[]")),
        env_entry("map-value", object(&[])),
    ])
}

fn assert_scope_rotation(entries: &[JsonValue]) {
    assert_eq!(
        entries.first(),
        Some(&scoped_entry(
            "user-var",
            string(&sealed("user-value", OLD_KEY)),
            string("user")
        )),
        "a user-scope entry keeps its placeholder ciphertext"
    );
    assert_eq!(
        entries.get(1),
        Some(&env_entry(
            "missing-scope",
            string(&sealed("missing-value", NEW_KEY))
        ))
    );
    assert_eq!(
        entries.get(2),
        Some(&scoped_entry(
            "global-scope",
            string(&sealed("global-value", NEW_KEY)),
            string("global")
        ))
    );
    assert_eq!(
        entries.get(3),
        Some(&scoped_entry(
            "odd-scope",
            string(&sealed("odd-value", NEW_KEY)),
            json("7")
        ))
    );
}

#[test]
fn env_scope_and_falsey_values() {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let result = reencrypt_global_env_values(stored(&scope_entries()).as_deref(), &old, &key);
    let entries = rotated_entries(result.unwrap());
    assert_scope_rotation(&entries);
    assert!(entries.contains(&env_entry("null-value", JsonValue::Null)));
    assert!(entries.contains(&env_entry("empty-value", string(""))));
    assert!(entries.contains(&env_entry("zero-value", json("0"))));
    assert!(entries.contains(&env_entry("false-value", JsonValue::Bool(false))));
    assert!(entries.contains(&env_entry("list-value", json("[]"))));
    assert!(entries.contains(&env_entry("map-value", object(&[]))));
    let inner = stored(&scope_entries()).unwrap();
    let entries = rotated_entries(
        reencrypt_global_env_values(stored(&string(&inner)).as_deref(), &old, &key).unwrap(),
    );
    assert_scope_rotation(&entries);
}

#[test]
fn env_scope_and_falsey_values_without_a_success_returns_none() {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let entries = array(vec![
        scoped_entry(
            "user-var",
            string(&sealed("user-value", OLD_KEY)),
            string("user"),
        ),
        env_entry("null-value", JsonValue::Null),
        env_entry("empty-value", string("")),
    ]);
    assert_eq!(
        reencrypt_global_env_values(stored(&entries).as_deref(), &old, &key).unwrap(),
        None
    );
    assert_eq!(
        reencrypt_global_env_values(Some("[]"), &old, &key).unwrap(),
        None
    );
    assert_eq!(
        reencrypt_global_env_values(Some("null"), &old, &key).unwrap(),
        None
    );
    assert_eq!(reencrypt_global_env_values(None, &old, &key).unwrap(), None);
}

#[test]
fn env_invalid_json_no_warning() {
    let old = cipher(OLD_KEY);
    assert_eq!(
        reencrypt_global_env_values(stored(&string("{not json")).as_deref(), &old, &new_key())
            .unwrap(),
        None
    );
    assert_eq!(
        reencrypt_global_env_values(stored(&string("plain")).as_deref(), &old, &new_key()).unwrap(),
        None
    );
    assert_eq!(
        reencrypt_global_env_values(Some(""), &old, &new_key()).unwrap(),
        None
    );
    assert_eq!(
        reencrypt_global_env_values(Some("false"), &old, &new_key()).unwrap(),
        None
    );
}

#[tokio::test]
async fn env_invalid_json_no_warning_through_rotation() {
    let store = only_server(stored(&string("{not json")));
    let (logs, guard) = capture_logs();
    let old = cipher(OLD_KEY);
    Rotation {
        store: &store,
        cipher: &old,
        new_key: &new_key(),
    }
    .servers("rotator")
    .await
    .unwrap();
    drop(guard);
    assert!(store.written().is_empty());
    let lines = logs.lines();
    assert_eq!(lines.len(), 1, "only the summary line is logged: {lines:?}");
    let summary = lines.first().unwrap();
    assert!(summary.contains("rotated 0 MCP server row(s)"));
}

#[test]
fn env_corrupt_preserved_warning() {
    let old = cipher(OLD_KEY);
    let entries = array(vec![
        env_entry("corrupt", string("not-a-ciphertext")),
        env_entry("valid", string(&sealed("valid-value", OLD_KEY))),
    ]);
    let entries = rotated_entries(
        reencrypt_global_env_values(stored(&entries).as_deref(), &old, &new_key()).unwrap(),
    );
    assert_eq!(
        entries.first(),
        Some(&env_entry("corrupt", string("not-a-ciphertext")))
    );
    assert_eq!(
        entries.get(1),
        Some(&env_entry("valid", string(&sealed("valid-value", NEW_KEY))))
    );
}

#[tokio::test]
async fn env_corrupt_preserved_warning_logs_the_name() {
    let entries = array(vec![env_entry("corrupt", string("not-a-ciphertext"))]);
    let store = only_server(stored(&entries));
    let (logs, guard) = capture_logs();
    let old = cipher(OLD_KEY);
    Rotation {
        store: &store,
        cipher: &old,
        new_key: &new_key(),
    }
    .servers("rotator")
    .await
    .unwrap();
    drop(guard);
    assert!(store.written().is_empty());
    let lines = logs.lines();
    assert_eq!(lines.len(), 2, "one warning and one summary: {lines:?}");
    let warning = lines.first().unwrap();
    assert!(warning.contains("could not decrypt global env var corrupt, skipping"));
}

#[tokio::test]
async fn env_no_success_no_write() {
    let entries = array(vec![
        scoped_entry(
            "user-var",
            string(&sealed("user-value", OLD_KEY)),
            string("user"),
        ),
        env_entry("null-value", JsonValue::Null),
    ]);
    let store = only_server(stored(&entries));
    let (logs, guard) = capture_logs();
    let old = cipher(OLD_KEY);
    Rotation {
        store: &store,
        cipher: &old,
        new_key: &new_key(),
    }
    .servers("rotator")
    .await
    .unwrap();
    drop(guard);
    assert!(store.written().is_empty());
    assert_eq!(logs.lines().len(), 1);
}

#[tokio::test]
async fn env_mixed_results() {
    let entries = array(vec![
        env_entry("valid", string(&sealed("valid-value", OLD_KEY))),
        env_entry("corrupt", string("not-a-ciphertext")),
        scoped_entry(
            "user-var",
            string(&sealed("user-value", OLD_KEY)),
            string("user"),
        ),
        env_entry("number-value", json("42")),
    ]);
    let store = only_server(stored(&entries));
    let (logs, guard) = capture_logs();
    let old = cipher(OLD_KEY);
    Rotation {
        store: &store,
        cipher: &old,
        new_key: &new_key(),
    }
    .servers("rotator")
    .await
    .unwrap();
    drop(guard);
    let writes = store.written();
    assert_eq!(writes.len(), 1);
    let super::support::Write::Server { update, .. } = &writes[0] else {
        panic!("the server row is written first");
    };
    let written = rotated_entries(update.env_vars.clone());
    assert_mixed_written(&written);
    let lines = logs.lines();
    assert_eq!(lines.len(), 2, "one warning and one summary: {lines:?}");
    let summary = lines.last().unwrap();
    assert!(summary.contains("rotated 1 MCP server row(s)"));
}

fn assert_mixed_written(written: &[JsonValue]) {
    assert_eq!(
        written.first(),
        Some(&env_entry("valid", string(&sealed("valid-value", NEW_KEY))))
    );
    assert_eq!(
        written.get(1),
        Some(&env_entry("corrupt", string("not-a-ciphertext")))
    );
    assert_eq!(
        written.get(2),
        Some(&scoped_entry(
            "user-var",
            string(&sealed("user-value", OLD_KEY)),
            string("user")
        ))
    );
    assert_eq!(written.get(3), Some(&env_entry("number-value", json("42"))));
}

#[test]
fn env_numeric_value_counts_as_rotated() {
    let entries = array(vec![env_entry("number-value", json("42"))]);
    let rotated =
        reencrypt_global_env_values(stored(&entries).as_deref(), &cipher(OLD_KEY), &new_key())
            .unwrap();
    assert_eq!(
        rotated_entries(rotated),
        vec![env_entry("number-value", json("42"))]
    );
}

#[test]
fn env_non_object_entry_errors() {
    let entries = array(vec![json("17")]);
    let result =
        reencrypt_global_env_values(stored(&entries).as_deref(), &cipher(OLD_KEY), &new_key());
    assert!(matches!(result, Err(RotationCodecError::Encoding(_))));
    let result = reencrypt_global_env_values(Some("17"), &cipher(OLD_KEY), &new_key());
    assert!(matches!(result, Err(RotationCodecError::Encoding(_))));
}
