#![cfg(test)]

use betterlite_db::mcp_server_encoding::credentials::CredentialBlob;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::secret_map::{SecretMap, encrypt_secret_map};
use secrecy::SecretString;

use super::super::codec::encrypt_credentials;
use super::super::error::RotationError;
use super::super::rotation::Rotation;
use super::super::store::{StoredUserCredentialRow, StoredUserEnvVarRow};
use super::support::{
    FakeStore, FixedNonces, NEW_KEY, OLD_KEY, TestCipher, Write, blob, capture_logs, cipher,
    empty_store, json, new_key, oauth_row, object, open, sealed, server_row, string,
};

type TestRotation<'rotation> = Rotation<'rotation, FakeStore, FixedNonces, TracingDiagnostics>;

fn test_rotation<'rotation>(
    store: &'rotation FakeStore,
    old: &'rotation TestCipher,
    key: &'rotation SecretString,
) -> TestRotation<'rotation> {
    Rotation {
        store,
        cipher: old,
        new_key: key,
    }
}

fn encrypted_credentials() -> JsonValue {
    let mut fields = CredentialBlob::new();
    fields.insert("client_id".to_owned(), string("client-id-value"));
    fields.insert("client_secret".to_owned(), string("client-secret-value"));
    encrypt_credentials(&mut fields, &cipher(OLD_KEY), None).unwrap();
    JsonValue::Object(fields)
}

fn written_server(writes: &[Write]) -> &StoredUpdate {
    let Write::Server { update, .. } = writes.first().expect("a server write") else {
        panic!("the first write is a server update");
    };
    update
}

type StoredUpdate = super::super::store::ServerUpdate;

fn decrypted_credentials(update: &StoredUpdate) -> CredentialBlob {
    let Some(JsonValue::String(text)) = update.credentials.as_ref() else {
        panic!("the rotated credentials are a JSON text string");
    };
    blob(text)
}

#[tokio::test]
async fn server_secret_map_warning() {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let mut store = empty_store();
    store.servers = vec![server_row(
        "s1",
        None,
        None,
        Some(string("not-a-secret-map")),
        Some(json(r#"{"X-Key":"plain-value"}"#)),
    )];
    let (logs, guard) = capture_logs();
    test_rotation(&store, &old, &key)
        .servers("rotator")
        .await
        .unwrap();
    drop(guard);
    let writes = store.written();
    assert_eq!(writes.len(), 1);
    let update = written_server(&writes);
    assert_eq!(
        update.static_headers, None,
        "the undecodable map is skipped"
    );
    let mut expected = SecretMap::new();
    expected.insert("X-Key".to_owned(), SecretString::from("plain-value"));
    assert_eq!(
        update.env,
        Some(JsonValue::String(
            encrypt_secret_map(&old, &expected, Some(&key)).unwrap()
        ))
    );
    let lines = logs.lines();
    assert_eq!(lines.len(), 2, "one warning and one summary: {lines:?}");
    let warning = lines.first().unwrap();
    assert!(warning.contains("Cannot rotate MCP static_headers for server s1"));
}

#[tokio::test]
async fn server_secret_map_json_text_roundtrip() {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let mut map = SecretMap::new();
    map.insert("X-Key".to_owned(), SecretString::from("plain-value"));
    let stored_headers = encrypt_secret_map(&old, &map, None).unwrap();
    let mut store = empty_store();
    store.servers = vec![server_row(
        "s1",
        None,
        None,
        Some(string(&stored_headers)),
        None,
    )];
    test_rotation(&store, &old, &key)
        .servers("rotator")
        .await
        .unwrap();
    let writes = store.written();
    assert_eq!(writes.len(), 1);
    let update = written_server(&writes);
    assert_eq!(
        update.static_headers,
        Some(JsonValue::String(
            encrypt_secret_map(&old, &map, Some(&key)).unwrap()
        )),
        "the stored JSON text of the ciphertext is rotated"
    );
}

#[tokio::test]
async fn server_secret_map_legacy_ciphertext() {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let sealed_map = sealed(r#"{"X-Key":"plain-value"}"#, OLD_KEY);
    let mut store = empty_store();
    store.servers = vec![server_row(
        "s1",
        None,
        None,
        None,
        Some(string(&sealed_map)),
    )];
    test_rotation(&store, &old, &key)
        .servers("rotator")
        .await
        .unwrap();
    let writes = store.written();
    assert_eq!(writes.len(), 1);
    let update = written_server(&writes);
    let mut map = SecretMap::new();
    map.insert("X-Key".to_owned(), SecretString::from("plain-value"));
    assert_eq!(
        update.env,
        Some(JsonValue::String(
            encrypt_secret_map(&old, &map, Some(&key)).unwrap()
        )),
        "a bare legacy ciphertext is decrypted rather than warned"
    );
}

#[tokio::test]
async fn server_empty_skip() {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let mut store = empty_store();
    store.servers = vec![
        server_row("empty", None, None, None, None),
        server_row(
            "empty-objects",
            Some(object(&[])),
            Some(json("[]")),
            Some(object(&[])),
            Some(object(&[])),
        ),
    ];
    test_rotation(&store, &old, &key)
        .servers("rotator")
        .await
        .unwrap();
    assert!(store.written().is_empty());
}

#[tokio::test]
async fn server_null_secret_map_skips() {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let mut store = empty_store();
    store.servers = vec![server_row(
        "s1",
        None,
        None,
        Some(JsonValue::Null),
        Some(JsonValue::Null),
    )];
    test_rotation(&store, &old, &key)
        .servers("rotator")
        .await
        .unwrap();
    assert!(store.written().is_empty());
}

#[tokio::test]
async fn server_json_text_empty_blob_is_rotated() {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let mut store = empty_store();
    store.servers = vec![server_row("s1", Some(string("{}")), None, None, None)];
    test_rotation(&store, &old, &key)
        .servers("rotator")
        .await
        .unwrap();
    let writes = store.written();
    assert_eq!(writes.len(), 1, "a JSON text empty blob is truthy");
    let update = written_server(&writes);
    assert_eq!(update.credentials, Some(JsonValue::String("{}".to_owned())));
}

#[tokio::test]
async fn server_updated_by() {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let mut store = empty_store();
    store.servers = vec![server_row(
        "s1",
        Some(encrypted_credentials()),
        None,
        None,
        None,
    )];
    test_rotation(&store, &old, &key)
        .servers("rotator-7")
        .await
        .unwrap();
    let writes = store.written();
    assert_eq!(writes.len(), 1);
    let Write::Server { server_id, update } = &writes[0] else {
        panic!("the server row is written");
    };
    assert_eq!(server_id, "s1");
    assert_eq!(update.updated_by, "rotator-7");
    let rotated = decrypted_credentials(update);
    let Some(JsonValue::String(client_id)) = rotated.get("client_id") else {
        panic!("client_id is a ciphertext string");
    };
    assert_eq!(open(client_id, NEW_KEY).as_deref(), Some("client-id-value"));
    assert_eq!(update.env_vars, None);
    assert_eq!(update.static_headers, None);
    assert_eq!(update.env, None);
}

#[tokio::test]
async fn server_before_oauth_clients() {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let mut store = empty_store();
    store.servers = vec![server_row(
        "s1",
        Some(encrypted_credentials()),
        None,
        None,
        None,
    )];
    store.oauth_clients = vec![
        oauth_row("o1", Some(encrypted_credentials())),
        oauth_row("o2", Some(encrypted_credentials())),
    ];
    let (logs, guard) = capture_logs();
    test_rotation(&store, &old, &key)
        .servers("rotator")
        .await
        .unwrap();
    drop(guard);
    let writes = store.written();
    assert_eq!(writes.len(), 3);
    assert!(matches!(&writes[0], Write::Server { server_id, .. } if server_id == "s1"));
    assert!(matches!(&writes[1], Write::OAuthClient { server_id, .. } if server_id == "o1"));
    assert!(matches!(&writes[2], Write::OAuthClient { server_id, .. } if server_id == "o2"));
    let lines = logs.lines();
    assert_eq!(lines.len(), 1, "only the summary line: {lines:?}");
    assert!(
        lines
            .first()
            .unwrap()
            .contains("rotated 1 MCP server row(s) and 2 OAuth-client row(s)")
    );
}

#[tokio::test]
async fn server_midstream_failure() {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let mut store = empty_store();
    store.servers = vec![
        server_row("s1", Some(encrypted_credentials()), None, None, None),
        server_row("s2", Some(string("not json")), None, None, None),
    ];
    store.oauth_clients = vec![oauth_row("o1", Some(encrypted_credentials()))];
    let result = test_rotation(&store, &old, &key).servers("rotator").await;
    assert!(matches!(result, Err(RotationError::Codec(_))));
    let writes = store.written();
    assert_eq!(writes.len(), 1, "only the first server is written");
    assert!(matches!(&writes[0], Write::Server { server_id, .. } if server_id == "s1"));
}

#[tokio::test]
async fn user_legacy_base64() {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let mut store = empty_store();
    store.user_credentials = vec![
        super::support::credential_row("u1", "s1", "aGVsbG8tdG9rZW4="),
        super::support::credential_row("u2", "s1", &sealed("encrypted-token", OLD_KEY)),
    ];
    let (logs, guard) = capture_logs();
    test_rotation(&store, &old, &key)
        .user_credentials()
        .await
        .unwrap();
    drop(guard);
    let writes = store.written();
    assert_eq!(writes.len(), 2);
    let Write::UserCredential { credential_b64, .. } = &writes[0] else {
        panic!("a user credential row is written");
    };
    assert_eq!(
        open(credential_b64, NEW_KEY).as_deref(),
        Some("hello-token")
    );
    let Write::UserCredential { credential_b64, .. } = &writes[1] else {
        panic!("a user credential row is written");
    };
    assert_eq!(
        open(credential_b64, NEW_KEY).as_deref(),
        Some("encrypted-token")
    );
    assert!(
        logs.lines()
            .first()
            .unwrap()
            .contains("rotated 2 row(s), skipped 0")
    );
}

#[tokio::test]
async fn user_corrupt_skip() {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let mut store = empty_store();
    store.user_credentials = vec![
        super::support::credential_row("u1", "s1", "__8="),
        super::support::credential_row("u2", "s1", "AA"),
    ];
    let (logs, guard) = capture_logs();
    test_rotation(&store, &old, &key)
        .user_credentials()
        .await
        .unwrap();
    drop(guard);
    assert!(store.written().is_empty());
    let lines = logs.lines();
    assert_eq!(lines.len(), 3, "two warnings and one summary: {lines:?}");
    assert!(lines.first().unwrap().contains("user_id=u1 server_id=s1"));
    assert!(lines.get(1).unwrap().contains("user_id=u2 server_id=s1"));
    assert!(
        lines
            .last()
            .unwrap()
            .contains("rotated 0 row(s), skipped 2")
    );
}

#[tokio::test]
async fn user_decode_error_classes() {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let mut store = empty_store();
    store.user_credentials = vec![
        super::support::credential_row("bad-length", "s1", "A"),
        super::support::credential_row("bad-utf8", "s1", "__8="),
        super::support::credential_row("non-ascii", "s1", "\u{e9}"),
        super::support::credential_row("empty-bytes", "s1", "***"),
    ];
    let (logs, guard) = capture_logs();
    test_rotation(&store, &old, &key)
        .user_credentials()
        .await
        .unwrap();
    drop(guard);
    let writes = store.written();
    assert_eq!(writes.len(), 1, "only the empty-bytes row rotates");
    let Write::UserCredential {
        user_id,
        credential_b64,
        ..
    } = &writes[0]
    else {
        panic!("a user credential row is written");
    };
    assert_eq!(user_id, "empty-bytes");
    assert_eq!(open(credential_b64, NEW_KEY).as_deref(), Some(""));
    assert!(
        logs.lines()
            .last()
            .unwrap()
            .contains("rotated 1 row(s), skipped 3")
    );
}

#[tokio::test]
async fn user_composite_update() {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let mut store = empty_store();
    store.user_credentials = vec![StoredUserCredentialRow {
        user_id: "user-9".to_owned(),
        server_id: "server-4".to_owned(),
        credential_b64: sealed("token", OLD_KEY),
    }];
    test_rotation(&store, &old, &key)
        .user_credentials()
        .await
        .unwrap();
    let writes = store.written();
    assert!(matches!(
        writes.first(),
        Some(Write::UserCredential { user_id, server_id, .. })
            if user_id == "user-9" && server_id == "server-4"
    ));
}

#[tokio::test]
async fn user_env_no_base64_fallback() {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let mut store = empty_store();
    store.user_env_vars = vec![super::support::env_row("u1", "s1", "aGVsbG8tdG9rZW4=")];
    let (logs, guard) = capture_logs();
    test_rotation(&store, &old, &key)
        .user_env_vars()
        .await
        .unwrap();
    drop(guard);
    assert!(store.written().is_empty());
    let lines = logs.lines();
    assert_eq!(lines.len(), 2, "one warning and one summary: {lines:?}");
    assert!(
        lines
            .first()
            .unwrap()
            .contains("could not decrypt env vars for user_id=u1 server_id=s1, skipping")
    );
}

#[tokio::test]
async fn user_env_empty_success() {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let mut store = empty_store();
    store.user_env_vars = vec![StoredUserEnvVarRow {
        user_id: "u1".to_owned(),
        server_id: "s1".to_owned(),
        values_b64: String::new(),
    }];
    test_rotation(&store, &old, &key)
        .user_env_vars()
        .await
        .unwrap();
    let writes = store.written();
    let Write::UserEnvVar { values_b64, .. } = writes.first().unwrap() else {
        panic!("a user env var row is written");
    };
    assert_eq!(open(values_b64, NEW_KEY).as_deref(), Some(""));
}

fn counts_store() -> FakeStore {
    let mut store = empty_store();
    store.user_credentials = vec![
        super::support::credential_row("u1", "s1", &sealed("one", OLD_KEY)),
        super::support::credential_row("u2", "s1", "AA"),
        super::support::credential_row("u3", "s1", &sealed("three", OLD_KEY)),
        super::support::credential_row("u4", "s1", "__8="),
    ];
    store.user_env_vars = vec![
        super::support::env_row("u1", "s1", &sealed("env-one", OLD_KEY)),
        super::support::env_row("u2", "s1", "AA"),
    ];
    store
}

#[tokio::test]
async fn user_rotation_counts() {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let store = counts_store();
    let (logs, guard) = capture_logs();
    test_rotation(&store, &old, &key)
        .user_credentials()
        .await
        .unwrap();
    test_rotation(&store, &old, &key)
        .user_env_vars()
        .await
        .unwrap();
    drop(guard);
    let lines = logs.lines();
    assert_eq!(
        lines.len(),
        5,
        "three warnings and two summaries: {lines:?}"
    );
    assert!(
        lines
            .get(2)
            .unwrap()
            .contains("rotated 2 row(s), skipped 2")
    );
    assert!(
        lines
            .last()
            .unwrap()
            .contains("rotated 1 row(s), skipped 1")
    );
}

#[tokio::test]
async fn user_rotation_partial_failure() {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let mut store = empty_store();
    store.user_credentials = vec![
        super::support::credential_row("u1", "s1", &sealed("one", OLD_KEY)),
        super::support::credential_row("u2", "s1", &sealed("two", OLD_KEY)),
        super::support::credential_row("u3", "s1", &sealed("three", OLD_KEY)),
    ];
    store.fail_at = Some(1);
    let (logs, guard) = capture_logs();
    let result = test_rotation(&store, &old, &key).user_credentials().await;
    drop(guard);
    assert!(matches!(result, Err(RotationError::Store(_))));
    let writes = store.written();
    assert_eq!(writes.len(), 1, "the first row stays written");
    assert!(matches!(&writes[0], Write::UserCredential { user_id, .. } if user_id == "u1"));
    assert!(
        logs.lines().is_empty(),
        "the summary is never logged after a failure"
    );
}
