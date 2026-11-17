use std::collections::BTreeMap;

use crate::budget_permission_rows::JsonData;
use crate::mcp::table_access::types::McpTableError;
use crate::mcp_user_credentials::rows::text_field;
use crate::mcp_user_credentials::types::CredentialError;
use crate::oauth_token_resolution::{TokenResolutionError, purge_server_oauth_credentials};
use crate::stored_cipher::DbCipher;

use super::fakes::{CacheEvent, FakeCaches};
use super::support::{
    FakeTable, TableMode, UNDECRYPTABLE, block_on, byok_row, cipher, credential_row, oauth_row,
    sealed, warnings_during,
};

fn oauth_payload_text(access: &str) -> String {
    format!(r#"{{"type":"oauth2","access_token":"{access}","refresh_token":"ref"}}"#)
}

fn four_row_table(cipher: &DbCipher) -> FakeTable {
    FakeTable::new(vec![
        oauth_row(cipher, "u1", "s1", &oauth_payload_text("tok-1")),
        oauth_row(cipher, "u2", "s1", &oauth_payload_text("tok-2")),
        byok_row(cipher, "u3", "s1"),
        oauth_row(cipher, "u4", "s2", &oauth_payload_text("tok-4")),
    ])
}

fn user_ids(table: &FakeTable) -> Vec<Option<String>> {
    table
        .stored()
        .iter()
        .map(|row| text_field(row, "user_id").map(str::to_owned))
        .collect()
}

fn pair(user_id: &str, server_id: &str) -> (String, String) {
    (user_id.to_owned(), server_id.to_owned())
}

fn invalidation(user_id: &str, server_id: &str) -> Vec<CacheEvent> {
    vec![
        CacheEvent::Discovery {
            server_id: server_id.to_owned(),
        },
        CacheEvent::Store {
            user_id: user_id.to_owned(),
            server_id: server_id.to_owned(),
        },
        CacheEvent::Legacy {
            user_id: user_id.to_owned(),
            server_id: server_id.to_owned(),
        },
    ]
}

#[test]
fn purge_deletes_only_the_oauth_rows_of_the_server() {
    let cipher = cipher();
    let table = four_row_table(&cipher);
    let caches = FakeCaches::working();
    let deleted = block_on(purge_server_oauth_credentials(
        &table, &cipher, &caches, "s1",
    ))
    .expect("the purge runs");
    assert_eq!(deleted, 2);
    assert_eq!(
        user_ids(&table),
        vec![Some("u3".to_owned()), Some("u4".to_owned())]
    );
    assert_eq!(table.deletes(), vec![pair("u1", "s1"), pair("u2", "s1")]);
}

#[test]
fn purge_invalidates_each_purged_users_token_cache() {
    let cipher = cipher();
    let table = four_row_table(&cipher);
    let caches = FakeCaches::working();
    block_on(purge_server_oauth_credentials(
        &table, &cipher, &caches, "s1",
    ))
    .expect("the purge runs");
    let mut expected = invalidation("u1", "s1");
    expected.extend(invalidation("u2", "s1"));
    assert_eq!(caches.events(), expected);
}

#[test]
fn purge_returns_zero_without_touching_a_byok_row() {
    let cipher = cipher();
    let table = FakeTable::new(vec![byok_row(&cipher, "u3", "s1")]);
    let caches = FakeCaches::working();
    let deleted = block_on(purge_server_oauth_credentials(
        &table, &cipher, &caches, "s1",
    ))
    .expect("the purge runs");
    assert_eq!(deleted, 0);
    assert_eq!(user_ids(&table), vec![Some("u3".to_owned())]);
    assert!(table.deletes().is_empty());
    assert!(caches.events().is_empty());
}

#[test]
fn purge_warns_when_fewer_rows_were_deleted_than_enumerated() {
    let cipher = cipher();
    let table = four_row_table(&cipher).with_mode(TableMode::SkipDeletes);
    let caches = FakeCaches::working();
    let warnings = warnings_during(|| {
        let deleted = block_on(purge_server_oauth_credentials(
            &table, &cipher, &caches, "s1",
        ))
        .expect("the purge runs");
        assert_eq!(deleted, 0);
    });
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("purge removed 0 OAuth credential row(s) but 2 were enumerated"));
}

#[test]
fn purge_deletes_a_mistyped_oauth2_payload() {
    let cipher = cipher();
    let mistyped = r#"{"type":"oauth2","access_token":"tok","scopes":"read"}"#;
    let table = FakeTable::new(vec![oauth_row(&cipher, "u1", "s1", mistyped)]);
    let caches = FakeCaches::working();
    let deleted = block_on(purge_server_oauth_credentials(
        &table, &cipher, &caches, "s1",
    ))
    .expect("the purge runs");
    assert_eq!(deleted, 1);
    assert!(table.stored().is_empty());
}

#[test]
fn purge_keeps_an_undecryptable_row() {
    let cipher = cipher();
    let table = FakeTable::new(vec![credential_row("u1", "s1", UNDECRYPTABLE)]);
    let caches = FakeCaches::working();
    let deleted = block_on(purge_server_oauth_credentials(
        &table, &cipher, &caches, "s1",
    ))
    .expect("the purge runs");
    assert_eq!(deleted, 0);
    assert!(table.deletes().is_empty());
}

#[test]
fn purge_reports_a_row_without_a_user_id() {
    let cipher = cipher();
    let row = JsonData::Object(BTreeMap::from([
        ("server_id".to_owned(), JsonData::String("s1".to_owned())),
        (
            "credential_b64".to_owned(),
            JsonData::String(sealed(&cipher, &oauth_payload_text("tok-1"))),
        ),
    ]));
    let table = FakeTable::new(vec![row]);
    let caches = FakeCaches::working();
    let error = block_on(purge_server_oauth_credentials(
        &table, &cipher, &caches, "s1",
    ))
    .expect_err("the purge reports the missing column");
    assert!(matches!(
        error,
        TokenResolutionError::Credential(CredentialError::MissingField { field: "user_id" })
    ));
}

#[test]
fn purge_reports_a_failing_enumeration() {
    let cipher = cipher();
    let table = FakeTable::in_mode(TableMode::FailReads);
    let caches = FakeCaches::working();
    let error = block_on(purge_server_oauth_credentials(
        &table, &cipher, &caches, "s1",
    ))
    .expect_err("the purge reports the failed read");
    assert!(matches!(
        error,
        TokenResolutionError::Table(McpTableError::MissingServerId)
    ));
}
