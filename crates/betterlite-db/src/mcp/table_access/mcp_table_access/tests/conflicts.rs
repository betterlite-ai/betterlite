use std::collections::BTreeMap;

use crate::mcp::table_access::McpIdentifierConflict;
use crate::mcp::table_access::filter::ServerFilter;
use crate::mcp::table_access::lock::{
    find_identifier_conflict, identifier_where, mcp_identifier_lock_keys,
};
use crate::mcp::table_access::rows::RowFields;
use crate::mcp::table_access::server_reads::identifier_field;
use crate::mcp::table_access::types::IdentifierField;
use crate::mcp_stored_contracts::JsonValue;

use super::support::{FakeStore, row};

#[tokio::test]
async fn an_alias_collision_is_reported_first() {
    let store = FakeStore::new(vec![
        row(r#"{"server_id":"name-owner","server_name":"name-taken","transport":"http"}"#),
        row(r#"{"server_id":"alias-owner","alias":"alias-taken","transport":"http"}"#),
    ]);
    let conflict = find_identifier_conflict(&store, Some("name-taken"), Some("alias-taken"), None)
        .await
        .expect("the fake store never fails")
        .expect("one identifier is taken");
    assert_eq!(
        conflict,
        McpIdentifierConflict {
            field: IdentifierField::Alias,
            value: "alias-taken".to_owned(),
            server_id: "alias-owner".to_owned(),
        }
    );
}

#[tokio::test]
async fn a_server_name_collision_names_its_column() {
    let store = FakeStore::new(vec![row(
        r#"{"server_id":"owner","server_name":"Shared","transport":"http"}"#,
    )]);
    let conflict = find_identifier_conflict(&store, Some("shared"), None, None)
        .await
        .expect("the fake store never fails")
        .expect("the name is taken ignoring case");
    assert_eq!(conflict.field, IdentifierField::ServerName);
    assert_eq!(conflict.server_id, "owner");
    assert_eq!(conflict.value, "shared");
}

#[tokio::test]
async fn draft_rows_never_collide() {
    let store = FakeStore::new(vec![row(
        r#"{"server_id":"draft","server_name":"shared","approval_status":"draft","transport":"http"}"#,
    )]);
    let conflict = find_identifier_conflict(&store, Some("shared"), None, None)
        .await
        .expect("the fake store never fails");
    assert_eq!(conflict, None);
}

#[tokio::test]
async fn rows_without_an_approval_status_still_collide() {
    let store = FakeStore::new(vec![row(
        r#"{"server_id":"legacy","server_name":"shared","transport":"http"}"#,
    )]);
    let conflict = find_identifier_conflict(&store, Some("shared"), None, None)
        .await
        .expect("the fake store never fails")
        .expect("a NULL approval_status predates the workflow and is kept");
    assert_eq!(conflict.server_id, "legacy");
}

#[tokio::test]
async fn the_excluded_server_id_never_collides_with_itself() {
    let store = FakeStore::new(vec![row(
        r#"{"server_id":"s1","server_name":"shared","transport":"http"}"#,
    )]);
    let excluded = find_identifier_conflict(&store, Some("shared"), None, Some("s1"))
        .await
        .expect("the fake store never fails");
    assert_eq!(excluded, None);
    let kept = find_identifier_conflict(&store, Some("shared"), None, None)
        .await
        .expect("the fake store never fails");
    assert!(kept.is_some());
}

#[tokio::test]
async fn empty_identifiers_do_not_query_for_a_collision() {
    let store = FakeStore::new(Vec::new());
    let conflict = find_identifier_conflict(&store, Some(""), Some(""), None)
        .await
        .expect("the fake store never fails");
    assert_eq!(conflict, None);
    assert!(store.finds().is_empty());
}

#[test]
fn the_identifier_predicate_keeps_null_rows_and_hides_drafts() {
    let (predicate, binds) = identifier_where("Shared", Some("s1")).predicate(1);
    assert_eq!(
        predicate,
        "((LOWER(\"server_name\") = LOWER($1) OR LOWER(\"alias\") = LOWER($1)) AND (\"approval_status\" IS NULL OR \"approval_status\" <> 'draft')) AND \"server_id\" <> $2"
    );
    assert_eq!(binds, vec!["Shared".to_owned(), "s1".to_owned()]);
    assert!(matches!(
        identifier_where("Shared", None),
        ServerFilter::Identifiers {
            exclude_server_id: None,
            ..
        }
    ));
}

#[test]
fn lock_keys_are_signed_big_endian_blake2b_digests() {
    assert_eq!(
        mcp_identifier_lock_keys(&[Some("a")]),
        vec![5_603_257_361_941_401_104_i64]
    );
    assert_eq!(
        mcp_identifier_lock_keys(&[Some("mcp")]),
        vec![8_703_809_971_163_426_483_i64]
    );
    assert_eq!(
        mcp_identifier_lock_keys(&[Some("alias")]),
        vec![-5_559_374_156_414_036_648_i64]
    );
}

#[test]
fn lock_keys_come_from_sorted_deduplicated_lowercased_identifiers() {
    let keys =
        mcp_identifier_lock_keys(&[Some("B"), None, Some("a"), Some(""), Some("b"), Some("A")]);
    assert_eq!(
        keys,
        vec![5_603_257_361_941_401_104_i64, 5_257_132_140_804_914_424_i64]
    );
    assert!(mcp_identifier_lock_keys(&[None, Some("")]).is_empty());
}

#[test]
fn the_identifier_field_reads_strings_only() {
    let fields = RowFields(BTreeMap::from([
        ("alias", JsonValue::String("alias-value".to_owned())),
        ("server_id", JsonValue::Null),
        (
            "server_name",
            JsonValue::Number(serde_json::Number::from(7_i32)),
        ),
    ]));
    assert_eq!(identifier_field(&fields, "server_name"), None);
    assert_eq!(identifier_field(&fields, "missing"), None);
    assert_eq!(
        identifier_field(&fields, "alias"),
        Some("alias-value".to_owned())
    );
}
