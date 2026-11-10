#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;

use crate::operation_ids::{ensure_unique_operation_ids, stable_operation_id};
use crate::schema_transforms::tree::JsonObject;

#[rstest]
#[case::single_method_suffix(
    "chat_completion",
    "/engines/{model}/chat/completions",
    &["POST"],
    "chat_completion_engines__model__chat_completions_post"
)]
#[case::single_lowercase_method(
    "chat_completion",
    "/v1/chat/completions",
    &["get"],
    "chat_completion_v1_chat_completions_get"
)]
#[case::two_methods_have_no_suffix(
    "chat_completion",
    "/chat/completions",
    &["POST", "GET"],
    "chat_completion_chat_completions"
)]
#[case::no_methods_have_no_suffix(
    "chat_completion",
    "/chat/completions",
    &[],
    "chat_completion_chat_completions"
)]
#[case::every_non_word_character_becomes_an_underscore(
    "a.b",
    "/c-d:e f",
    &[],
    "a_b_c_d_e_f"
)]
fn stable_operation_ids_replace_non_word_characters_and_suffix_only_one_method(
    #[case] name: &str,
    #[case] path_format: &str,
    #[case] methods: &[&str],
    #[case] expected: &str,
) {
    assert_eq!(stable_operation_id(name, path_format, methods), expected);
}

fn document(json: &str) -> JsonObject {
    serde_json::from_str(json).expect("the test document is valid JSON")
}

#[test]
fn every_occurrence_of_a_repeated_id_is_rewritten_in_document_order() {
    let mut openapi = document(
        r#"{"paths": {
            "/a": {"get": {"operationId": "dup"}, "post": {"operationId": "dup"}},
            "/b": {"get": {"operationId": "dup"}, "post": {"operationId": "taken"}},
            "/c": {"get": {"operationId": "taken"}, "x-custom": {"operationId": "dup"}}
        }}"#,
    );
    ensure_unique_operation_ids(&mut openapi, None);
    let expected = document(
        r#"{"paths": {
            "/a": {"get": {"operationId": "dup_get"}, "post": {"operationId": "dup_post"}},
            "/b": {"get": {"operationId": "dup_get_2"}, "post": {"operationId": "taken_post"}},
            "/c": {"get": {"operationId": "taken_get"}, "x-custom": {"operationId": "dup"}}
        }}"#,
    );
    assert_eq!(openapi, expected);
}

#[test]
fn a_reserved_id_is_rewritten_and_the_reserved_set_keeps_every_used_id() {
    let mut openapi = document(r#"{"paths": {"/a": {"get": {"operationId": "hello"}}}}"#);
    let mut reserved = BTreeSet::from(["hello".to_owned()]);
    ensure_unique_operation_ids(&mut openapi, Some(&mut reserved));
    assert_eq!(
        openapi,
        document(r#"{"paths": {"/a": {"get": {"operationId": "hello_get"}}}}"#)
    );
    assert_eq!(
        reserved,
        BTreeSet::from(["hello".to_owned(), "hello_get".to_owned()])
    );
}

#[test]
fn further_clashes_count_up_after_the_method() {
    let mut openapi = document(
        r#"{"paths": {
            "/a": {"get": {"operationId": "dup"}},
            "/b": {"get": {"operationId": "dup"}},
            "/c": {"get": {"operationId": "dup"}}
        }}"#,
    );
    ensure_unique_operation_ids(&mut openapi, None);
    assert_eq!(
        openapi,
        document(
            r#"{"paths": {
                "/a": {"get": {"operationId": "dup_get"}},
                "/b": {"get": {"operationId": "dup_get_2"}},
                "/c": {"get": {"operationId": "dup_get_3"}}
            }}"#
        )
    );
}

#[test]
fn an_earlier_operation_that_already_uses_the_method_suffix_pushes_the_counter() {
    let mut openapi = document(
        r#"{"paths": {
            "/a": {"get": {"operationId": "dup_get"}},
            "/b": {"get": {"operationId": "dup"}},
            "/c": {"get": {"operationId": "dup"}}
        }}"#,
    );
    ensure_unique_operation_ids(&mut openapi, None);
    assert_eq!(
        openapi,
        document(
            r#"{"paths": {
                "/a": {"get": {"operationId": "dup_get"}},
                "/b": {"get": {"operationId": "dup_get_2"}},
                "/c": {"get": {"operationId": "dup_get_3"}}
            }}"#
        )
    );
}

#[test]
fn only_a_trailing_known_method_is_stripped_before_the_new_suffix() {
    let mut openapi = document(
        r#"{"paths": {
            "/a": {"get": {"operationId": "foo_GET"}},
            "/b": {"get": {"operationId": "foo_GET"}},
            "/c": {"post": {"operationId": "foo_bar"}},
            "/d": {"post": {"operationId": "foo_bar"}},
            "/e": {"get": {"operationId": "foo_post"}},
            "/f": {"get": {"operationId": "foo_post"}}
        }}"#,
    );
    ensure_unique_operation_ids(&mut openapi, None);
    assert_eq!(
        openapi,
        document(
            r#"{"paths": {
                "/a": {"get": {"operationId": "foo_GET_get"}},
                "/b": {"get": {"operationId": "foo_GET_get_2"}},
                "/c": {"post": {"operationId": "foo_bar_post"}},
                "/d": {"post": {"operationId": "foo_bar_post_2"}},
                "/e": {"get": {"operationId": "foo_get"}},
                "/f": {"get": {"operationId": "foo_get_2"}}
            }}"#
        )
    );
}

#[test]
fn unique_string_ids_and_everything_else_stay_untouched() {
    let mut openapi = document(
        r#"{"paths": {
            "/a": {"get": {"operationId": 5}, "summary": {"operationId": "dup"}, "post": {"operationId": "only"}},
            "/b": "not a path item",
            "/c": {"get": "not an operation"}
        }}"#,
    );
    let expected = openapi.clone();
    ensure_unique_operation_ids(&mut openapi, None);
    assert_eq!(openapi, expected);
}

#[test]
fn an_empty_document_and_a_missing_paths_key_are_fine() {
    let mut empty = JsonObject::new();
    ensure_unique_operation_ids(&mut empty, None);
    assert!(empty.is_empty());
    let mut document_without_paths = document(r#"{"openapi": "3.1.0"}"#);
    ensure_unique_operation_ids(&mut document_without_paths, None);
    assert_eq!(
        serde_json::to_string(&document_without_paths).expect("the document serializes"),
        r#"{"openapi":"3.1.0"}"#
    );
}
