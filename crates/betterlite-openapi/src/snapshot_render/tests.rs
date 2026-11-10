#![cfg(test)]

use std::fs;

use crate::document_builder::{DocumentInputs, build_document};
use crate::schema_transforms::tree::{JsonObject, JsonTree};
use crate::snapshot_render::{check_snapshot, render_snapshot};

const DOCUMENT_INPUTS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/document_inputs.json"
);
const GENERATED_SNAPSHOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/openapi.generated.json"
);

fn document(json: &str) -> JsonObject {
    serde_json::from_str(json).expect("the test document is valid JSON")
}

#[test]
fn the_committed_generated_fixture_is_what_the_exporter_writes() {
    let inputs: DocumentInputs = serde_json::from_str(
        &fs::read_to_string(DOCUMENT_INPUTS).expect("the document inputs fixture is readable"),
    )
    .expect("the document inputs fixture is valid");
    let committed =
        fs::read_to_string(GENERATED_SNAPSHOT).expect("the generated snapshot fixture is readable");
    assert_eq!(render_snapshot(&build_document(&inputs)), committed);
}

#[test]
fn the_snapshot_sorts_keys_at_every_depth_and_keeps_array_order() {
    let rendered = render_snapshot(&document(
        r#"{"z": "value", "a": {"c": true, "b": [{"y": 2, "x": 1}]}, "m": [], "f": {}}"#,
    ));
    let expected = "{\n\
                    \x20 \"a\": {\n\
                    \x20   \"b\": [\n\
                    \x20     {\n\
                    \x20       \"x\": 1,\n\
                    \x20       \"y\": 2\n\
                    \x20     }\n\
                    \x20   ],\n\
                    \x20   \"c\": true\n\
                    \x20 },\n\
                    \x20 \"f\": {},\n\
                    \x20 \"m\": [],\n\
                    \x20 \"z\": \"value\"\n\
                    }\n";
    assert_eq!(rendered, expected);
}

#[test]
fn strings_are_escaped_as_json_and_non_ascii_stays_utf8() {
    let mut openapi = JsonObject::new();
    openapi.insert(
        "text".to_owned(),
        JsonTree::Text("a\"b\\c\nd\te\u{08}\u{0C}\u{0D}\u{0}g\u{2014}".to_owned()),
    );
    let rendered = render_snapshot(&openapi);
    let expected = "{\n  \"text\": \"a\\\"b\\\\c\\nd\\te\\bf\\f\\r\\u0000g\u{2014}\"\n}\n";
    assert_eq!(rendered, expected);
}

#[test]
fn rendering_round_trips_through_serde_json() {
    let original = document(
        r#"{"openapi": "3.1.0", "paths": {"/a": {"get": {"parameters": [{"required": true, "n": 1.5}]}}}}"#,
    );
    let rendered = render_snapshot(&original);
    assert_eq!(document(&rendered), original);
    assert!(rendered.ends_with("}\n"));
    assert!(!rendered.ends_with("\n\n"));
    assert_eq!(render_snapshot(&JsonObject::new()), "{}\n");
}

#[test]
fn check_snapshot_is_ok_only_on_byte_equality() {
    assert_eq!(check_snapshot("{}\n", "{}\n"), Ok(()));
    let drift = check_snapshot("{}", "{}\n").expect_err("a trailing newline is a difference");
    assert!(drift.added.is_empty());
    assert!(drift.removed.is_empty());
    assert!(drift.changed.is_empty());
    assert_eq!(drift.unreadable, None);
    assert_eq!(
        drift.message(),
        "the committed snapshot differs from the rendered document"
    );
}

#[test]
fn check_snapshot_names_added_removed_and_changed_paths() {
    let committed = r#"{"paths": {"/a": {}, "/b": {"get": {}}, "/c": {}}}"#;
    let rendered = document(r#"{"paths": {"/a": {}, "/b": {"get": {"x": 1}}, "/d": {}}}"#);
    let drift =
        check_snapshot(committed, &render_snapshot(&rendered)).expect_err("the documents differ");
    assert_eq!(drift.added, vec!["/d".to_owned()]);
    assert_eq!(drift.removed, vec!["/c".to_owned()]);
    assert_eq!(drift.changed, vec!["/b".to_owned()]);
    assert_eq!(drift.unreadable, None);
    let message = drift.message();
    assert!(message.contains("added paths: /d"), "{message}");
    assert!(message.contains("removed paths: /c"), "{message}");
    assert!(message.contains("changed paths: /b"), "{message}");
}

#[test]
fn check_snapshot_reports_a_committed_file_that_is_not_json() {
    let drift = check_snapshot("not json", "{}\n").expect_err("the committed file is unreadable");
    assert!(drift.added.is_empty());
    assert!(drift.removed.is_empty());
    assert!(drift.changed.is_empty());
    let reason = drift
        .unreadable
        .as_deref()
        .expect("the drift names the parse failure");
    assert!(reason.contains("the committed file is not valid JSON"));
    assert!(drift.message().contains("not valid JSON"));
}

#[test]
fn a_change_outside_paths_still_fails_the_check() {
    let drift = check_snapshot(r#"{"info": {"title": "old"}}"#, "{\n  \"info\": {}\n}\n")
        .expect_err("the documents differ");
    assert!(drift.added.is_empty() && drift.removed.is_empty() && drift.changed.is_empty());
}
