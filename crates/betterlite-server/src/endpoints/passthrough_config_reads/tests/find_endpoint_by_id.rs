#![cfg(test)]

use betterlite_db::budget_permission_rows::JsonData;

use crate::endpoints::passthrough_config_reads::db_read::find_endpoint_by_id;
use crate::endpoints::passthrough_config_reads::record::EndpointRecordError;
use crate::endpoints::passthrough_config_reads::support::json;

const A: &str = r#"{"id": "a", "path": "/a", "target": "https://a"}"#;
const B: &str = r#"{"id": "b", "path": "/b", "target": "https://b"}"#;

fn entries(raw: &[&str]) -> Vec<JsonData> {
    let mut values = Vec::with_capacity(raw.len());
    for entry in raw {
        values.push(json(entry));
    }
    values
}

#[test]
fn the_matching_entry_is_returned() {
    let found = find_endpoint_by_id(&entries(&[A, B]), "b")
        .expect("a lookup")
        .expect("the matching endpoint");
    assert_eq!(found.id.as_deref(), Some("b"));
    assert_eq!(found.path, "/b");
}

#[test]
fn an_unknown_id_gives_none() {
    let found = find_endpoint_by_id(&entries(&[A, B]), "missing").expect("a lookup");
    assert_eq!(found, None);
}

#[test]
fn an_empty_list_gives_none() {
    let found = find_endpoint_by_id(&[], "a").expect("a lookup");
    assert_eq!(found, None);
}

#[test]
fn a_non_object_entry_is_skipped() {
    let found = find_endpoint_by_id(&entries(&[r#""text""#, "7", "null", B]), "b")
        .expect("a lookup")
        .expect("the matching endpoint");
    assert_eq!(found.path, "/b");
}

#[test]
fn an_entry_without_an_id_never_matches() {
    let found = find_endpoint_by_id(&entries(&[r#"{"path": "/a", "target": "https://a"}"#]), "a")
        .expect("a lookup");
    assert_eq!(found, None);
}

#[test]
fn a_malformed_entry_is_an_error() {
    let error = find_endpoint_by_id(&entries(&[r#"{"path": "/broken"}"#]), "a")
        .expect_err("a malformed record");
    assert!(matches!(error, EndpointRecordError::Decode(_)));
}

#[test]
fn a_malformed_entry_before_the_match_is_an_error() {
    let error = find_endpoint_by_id(&entries(&[r#"{"path": "/broken"}"#, B]), "b")
        .expect_err("a malformed record");
    assert!(matches!(error, EndpointRecordError::Decode(_)));
}

#[test]
fn a_malformed_entry_after_the_match_is_not_read() {
    let found = find_endpoint_by_id(&entries(&[B, r#"{"path": "/broken"}"#]), "b")
        .expect("a lookup")
        .expect("the matching endpoint");
    assert_eq!(found.path, "/b");
}

#[test]
fn the_found_endpoint_is_marked_as_stored() {
    let found = find_endpoint_by_id(
        &entries(&[r#"{"id": "b", "path": "/b", "target": "https://b", "is_from_config": true}"#]),
        "b",
    )
    .expect("a lookup")
    .expect("the matching endpoint");
    assert!(!found.is_from_config.0);
}
