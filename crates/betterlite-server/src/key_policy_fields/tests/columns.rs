#![cfg(test)]

use std::path::PathBuf;

use betterlite_db::identity_rows::Payload;
use rstest::rstest;

use super::object;
use crate::key_fields::policy::{PolicyOperation, PolicyRequestBody};
use crate::key_policy_fields::columns::{
    ColumnError, decode_json_column_bytes, decode_json_string_column, effective_key_after_update,
    update_policy_request, verification_token_from_row,
};

#[test]
fn json_columns_only() {
    let parsed = decode_json_string_column("router_settings", &super::json(r#"{"a":1}"#)).unwrap();
    assert_eq!(parsed, super::json(r#"{"a":1}"#));
    let windows = decode_json_string_column(
        "budget_limits",
        &super::json(r#"[{"budget_duration":"1d"}]"#),
    )
    .unwrap();
    assert_eq!(windows, super::json(r#"[{"budget_duration":"1d"}]"#));
    // a column outside the pair keeps its text, and a non-string never parses
    let text = super::json(r#"{"not":"decoded"}"#);
    assert_eq!(decode_json_string_column("metadata", &text).unwrap(), text);
    assert_eq!(
        decode_json_string_column("router_settings", &super::json("null")).unwrap(),
        super::json("null")
    );
    assert_eq!(
        decode_json_string_column("budget_limits", &super::json("7")).unwrap(),
        super::json("7")
    );
}

#[test]
fn malformed_column_json() {
    let error = decode_json_string_column("budget_limits", &super::json("[")).unwrap_err();
    assert!(matches!(&error, ColumnError::Json { column, .. } if column == "budget_limits"));
    let bytes = decode_json_column_bytes("router_settings", &[0xff, 0xfe]).unwrap_err();
    assert!(matches!(&bytes, ColumnError::Utf8 { column, .. } if column == "router_settings"));
    assert!(decode_json_column_bytes("router_settings", b"{\"a\":").is_err());
    assert_eq!(
        decode_json_column_bytes("router_settings", b"{}").unwrap(),
        super::json("{}")
    );
}

#[test]
fn organization_id_null_wins() {
    let row = object(r#"{"organization_id":null,"org_id":"org-1","key_alias":"alias"}"#);
    let token = verification_token_from_row(&row).unwrap();
    assert_eq!(token.org_id, None);
    assert_eq!(token.key_alias.as_deref(), Some("alias"));
    // a row without the key falls back, and a populated organization_id also wins
    let fallback = verification_token_from_row(&object(r#"{"org_id":"org-1"}"#)).unwrap();
    assert_eq!(fallback.org_id.as_deref(), Some("org-1"));
    let populated =
        verification_token_from_row(&object(r#"{"organization_id":"org-2","org_id":"org-1"}"#))
            .unwrap();
    assert_eq!(populated.org_id.as_deref(), Some("org-2"));
}

#[test]
fn effective_patch_clears_permission() {
    let existing = verification_token_from_row(&object(
        r#"{"key_alias":"old","object_permission":{"object_permission_id":"perm-1"},"router_settings":{"a":1}}"#,
    ))
    .unwrap();
    assert!(existing.object_permission.is_some());
    let patch = object(r#"{"key_alias":"new","router_settings":"{\"b\":2}"}"#);
    let effective = effective_key_after_update(&existing, &patch).unwrap();
    assert_eq!(effective.key_alias.as_deref(), Some("new"));
    assert!(effective.object_permission.is_none());
    assert_eq!(
        effective.router_settings,
        Some(Payload(std::collections::BTreeMap::from([(
            "b".to_owned(),
            super::json("2")
        )])))
    );
    // the stored row keeps its own values
    assert_eq!(existing.key_alias.as_deref(), Some("old"));
    assert!(existing.object_permission.is_some());
}

#[test]
fn policy_operation_and_original_request() {
    let existing = verification_token_from_row(&object(
        r#"{"key_alias":"old","token":"hashed","object_permission":{"object_permission_id":"perm-1"}}"#,
    ))
    .unwrap();
    let request = super::update_request(r#"{"key_alias":"new","spend":2.5}"#);
    let patch = object(r#"{"key_alias":"new"}"#);
    let record = update_policy_request(
        PolicyOperation::Update,
        &existing,
        &patch,
        PolicyRequestBody::Update(request),
    )
    .unwrap();
    assert_eq!(record.operation, PolicyOperation::Update);
    let untouched = record.existing_key.unwrap();
    assert_eq!(untouched.key_alias.as_deref(), Some("old"));
    assert!(untouched.object_permission.is_some());
    assert_eq!(record.effective_key.key_alias.as_deref(), Some("new"));
    assert!(record.effective_key.object_permission.is_none());
    assert!(matches!(record.request, PolicyRequestBody::Update(_)));
}

#[rstest]
fn replays_the_key_policy_json_column_corpus(
    #[files("fuzz/corpus/key_policy_json_column/*")] path: PathBuf,
) {
    let bytes = std::fs::read(&path).unwrap();
    let separator = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .expect("every seed names its column on the first line");
    let column = std::str::from_utf8(&bytes[..separator]).unwrap();
    let stored = &bytes[separator + 1..];
    let expected_error = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|name| name.contains("-invalid"));
    assert_eq!(
        decode_json_column_bytes(column, stored).is_err(),
        expected_error,
        "{path:?}"
    );
}
