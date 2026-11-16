use rstest::rstest;

use crate::budget_permission_rows::JsonData;

use super::super::store::store_user_credential;
use super::super::types::{CredentialError, CredentialType};
use super::super::{
    list_server_user_credentials, list_user_oauth_credentials, server_user_credential_item,
};
use super::support::{
    FakeTable, UPDATED_AT, block_on, cipher, credential_row, exposed, warnings_during,
};
use super::{ENCRYPTED_OAUTH, ENCRYPTED_SECRET, LEGACY_OAUTH, UNDECRYPTABLE};

#[test]
fn item_reports_an_oauth_row_without_its_secret() {
    let row = credential_row("u1", "s1", ENCRYPTED_OAUTH);
    let item = server_user_credential_item(&row, &cipher()).expect("the row reads");
    assert_eq!(item.user_id, "u1");
    assert_eq!(item.credential_type, CredentialType::Oauth2);
    assert_eq!(
        item.expires_at.as_deref(),
        Some("2030-01-02T03:04:05+00:00")
    );
    assert_eq!(
        item.connected_at.as_deref(),
        Some("2024-01-02T03:04:05+00:00")
    );
    assert_eq!(item.updated_at, UPDATED_AT);
    let written = serde_json::to_string(&item).expect("the item serializes");
    assert!(!written.contains("tok-123"));
    assert!(!written.contains("ref-456"));
}

#[test]
fn item_reports_a_byok_row_without_expiry_or_connection() {
    let row = credential_row("u1", "s1", ENCRYPTED_SECRET);
    let item = server_user_credential_item(&row, &cipher()).expect("the row reads");
    assert_eq!(item.credential_type, CredentialType::Byok);
    assert_eq!(item.expires_at, None);
    assert_eq!(item.connected_at, None);
    assert_eq!(item.updated_at, UPDATED_AT);
    let written = serde_json::to_string(&item).expect("the item serializes");
    assert!(!written.contains("secret-value"));
}

#[test]
fn item_reports_an_undecryptable_row_as_byok() {
    let row = credential_row("u1", "s1", UNDECRYPTABLE);
    let item = server_user_credential_item(&row, &cipher()).expect("the row reads");
    assert_eq!(item.credential_type, CredentialType::Byok);
    assert_eq!(item.expires_at, None);
}

#[test]
fn item_reports_a_legacy_oauth_row_as_oauth2() {
    let row = credential_row("u1", "s1", LEGACY_OAUTH);
    let item = server_user_credential_item(&row, &cipher()).expect("the row reads");
    assert_eq!(item.credential_type, CredentialType::Oauth2);
    assert_eq!(
        item.expires_at.as_deref(),
        Some("2030-01-02T03:04:05+00:00")
    );
}

#[rstest]
#[case::user_id("user_id")]
#[case::updated_at("updated_at")]
#[case::credential_b64("credential_b64")]
fn item_requires_every_column(#[case] missing: &str) {
    let mut row = credential_row("u1", "s1", ENCRYPTED_SECRET);
    let JsonData::Object(fields) = &mut row else {
        panic!("the fixture row is an object");
    };
    drop(fields.remove(missing));
    let failed = server_user_credential_item(&row, &cipher()).expect_err("the column is missing");
    let CredentialError::MissingField { field } = failed else {
        panic!("a missing column is reported by name");
    };
    assert_eq!(field, missing);
}

#[test]
fn list_server_credentials_keeps_only_the_given_server() {
    let table = FakeTable::new(vec![
        credential_row("u1", "s1", ENCRYPTED_OAUTH),
        credential_row("u2", "s1", ENCRYPTED_SECRET),
        credential_row("u1", "s2", ENCRYPTED_OAUTH),
    ]);
    let items =
        block_on(list_server_user_credentials(&table, &cipher(), "s1")).expect("the listing runs");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].user_id, "u1");
    assert_eq!(items[0].credential_type, CredentialType::Oauth2);
    assert_eq!(items[1].user_id, "u2");
    assert_eq!(items[1].credential_type, CredentialType::Byok);
    let written = serde_json::to_string(&items).expect("the items serialize");
    assert!(!written.contains("tok-123"));
    assert!(!written.contains("secret-value"));
}

#[test]
fn list_user_oauth_credentials_tags_each_payload_with_its_server() {
    let table = FakeTable::new(vec![
        credential_row("u1", "s1", ENCRYPTED_OAUTH),
        credential_row("u1", "s2", ENCRYPTED_SECRET),
        credential_row("u2", "s3", ENCRYPTED_OAUTH),
    ]);
    let payloads =
        block_on(list_user_oauth_credentials(&table, &cipher(), "u1")).expect("the listing runs");
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0].server_id.as_deref(), Some("s1"));
    assert_eq!(exposed(payloads[0].access_token.as_ref()), Some("tok-123"));
    assert_eq!(
        payloads[0].scopes,
        Some(vec!["read".to_owned(), "write".to_owned()])
    );
}

#[test]
fn list_user_oauth_credentials_warns_for_each_undecryptable_row() {
    let table = FakeTable::new(vec![
        credential_row("u1", "s1", UNDECRYPTABLE),
        credential_row("u1", "s2", UNDECRYPTABLE),
        credential_row("u1", "s3", ENCRYPTED_OAUTH),
    ]);
    let payloads = warnings_during(|| {
        let listed = block_on(list_user_oauth_credentials(&table, &cipher(), "u1"));
        let payloads = listed.expect("the listing runs");
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].server_id.as_deref(), Some("s3"));
    });
    assert_eq!(payloads.len(), 2);
    assert!(payloads[0].contains("server_id=s1"));
    assert!(payloads[1].contains("server_id=s2"));
}

#[test]
fn list_user_oauth_credentials_keeps_a_byok_row_without_a_warning() {
    let table = FakeTable::new(vec![credential_row("u1", "s1", ENCRYPTED_SECRET)]);
    let payloads = warnings_during(|| {
        let listed = block_on(list_user_oauth_credentials(&table, &cipher(), "u1"));
        assert!(listed.expect("the listing runs").is_empty());
    });
    assert!(payloads.is_empty());
}

const MISTYPED_PAYLOAD: &str = r#"{"type":"oauth2","access_token":"tok","scopes":"read"}"#;

fn mistyped_table() -> FakeTable {
    let table = FakeTable::new(Vec::new());
    block_on(store_user_credential(
        &table,
        &cipher(),
        "u1",
        "s1",
        MISTYPED_PAYLOAD,
    ))
    .expect("the store runs");
    table
}

#[test]
fn item_reports_a_mistyped_oauth_payload_as_an_error_and_not_as_byok() {
    let row = mistyped_table().stored().remove(0);
    let failed = server_user_credential_item(&row, &cipher());
    assert!(matches!(failed, Err(CredentialError::Json(_))));
}

#[test]
fn list_server_credentials_fails_on_a_mistyped_oauth_payload() {
    let table = mistyped_table();
    let failed = block_on(list_server_user_credentials(&table, &cipher(), "s1"));
    assert!(matches!(failed, Err(CredentialError::Json(_))));
}

#[test]
fn list_user_oauth_credentials_fails_on_a_mistyped_oauth_payload() {
    let table = mistyped_table();
    let failed = block_on(list_user_oauth_credentials(&table, &cipher(), "u1"));
    assert!(matches!(failed, Err(CredentialError::Json(_))));
}
