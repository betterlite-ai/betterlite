#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::{MAX_MERGE_DEPTH, MergePatchError, apply_json_merge_patch};

fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

// exactly `levels` nested objects, each one under "a", with `leaf` at the innermost key
fn nested_object_patch(levels: usize, leaf: JsonValue) -> JsonValue {
    let mut value = leaf;
    for _ in 0..levels {
        let mut outer: BTreeMap<String, JsonValue> = BTreeMap::new();
        outer.insert("a".to_owned(), value);
        value = JsonValue::Object(outer.into_iter().collect());
    }
    value
}

fn nested_array(depth: usize) -> JsonValue {
    let mut value = json(r#""deep""#);
    for _ in 0..depth {
        value = JsonValue::Array(vec![value]);
    }
    value
}

#[rstest]
#[case::overwrite_a_key(r#"{"a":"b"}"#, r#"{"a":"c"}"#, r#"{"a":"c"}"#)]
#[case::add_a_key(r#"{"a":"b"}"#, r#"{"b":"c"}"#, r#"{"a":"b","b":"c"}"#)]
#[case::remove_a_key(r#"{"a":"b"}"#, r#"{"a":null}"#, "{}")]
#[case::remove_one_of_two_keys(r#"{"a":"b","b":"c"}"#, r#"{"a":null}"#, r#"{"b":"c"}"#)]
#[case::replace_an_array_with_a_scalar(r#"{"a":["b"]}"#, r#"{"a":"c"}"#, r#"{"a":"c"}"#)]
#[case::replace_a_scalar_with_an_array(r#"{"a":"c"}"#, r#"{"a":["b"]}"#, r#"{"a":["b"]}"#)]
#[case::merge_nested_objects(
    r#"{"a":{"b":"c"}}"#,
    r#"{"a":{"b":"d","c":null}}"#,
    r#"{"a":{"b":"d"}}"#
)]
#[case::replace_an_array_of_objects(r#"{"a":[{"b":"c"}]}"#, r#"{"a":[1]}"#, r#"{"a":[1]}"#)]
#[case::replace_an_array_target(r#"["a","b"]"#, r#"["c","d"]"#, r#"["c","d"]"#)]
#[case::replace_the_target_with_an_array(r#"{"a":"b"}"#, r#"["c"]"#, r#"["c"]"#)]
#[case::replace_the_target_with_null(r#"{"a":"foo"}"#, "null", "null")]
#[case::replace_the_target_with_a_string(r#"{"a":"foo"}"#, r#""bar""#, r#""bar""#)]
#[case::keep_a_null_key_of_the_target(r#"{"e":null}"#, r#"{"a":1}"#, r#"{"e":null,"a":1}"#)]
#[case::merge_into_an_array_target("[1,2]", r#"{"a":"b","c":null}"#, r#"{"a":"b"}"#)]
#[case::a_nested_null_creates_the_parent_object(
    "{}",
    r#"{"a":{"bb":{"ccc":null}}}"#,
    r#"{"a":{"bb":{}}}"#
)]
fn merge_patch_cases_follow_rfc_7386(
    #[case] target: &str,
    #[case] patch: &str,
    #[case] expected: &str,
) {
    let result = apply_json_merge_patch(&json(target), &json(patch)).unwrap();
    assert_eq!(result, json(expected));
}

#[test]
fn the_target_is_never_mutated() {
    let target = json(r#"{"a":{"b":1,"c":2},"d":3}"#);
    let patch = json(r#"{"a":{"b":9,"c":null},"d":null,"e":4}"#);
    let result = apply_json_merge_patch(&target, &patch).unwrap();
    assert_eq!(result, json(r#"{"a":{"b":9},"e":4}"#));
    assert_eq!(target, json(r#"{"a":{"b":1,"c":2},"d":3}"#));
}

#[test]
fn the_maximum_depth_is_sixty_four() {
    assert_eq!(MAX_MERGE_DEPTH, 64);
}

#[test]
fn sixty_four_nested_object_patches_are_applied() {
    let patch = nested_object_patch(64, json("1"));
    let result = apply_json_merge_patch(&json("{}"), &patch).unwrap();
    assert_eq!(result, patch);
}

#[test]
fn sixty_five_nested_object_patches_fail_with_too_deep() {
    let patch = nested_object_patch(65, json("1"));
    let error = apply_json_merge_patch(&json("{}"), &patch).unwrap_err();
    assert_eq!(error, MergePatchError::TooDeep { max: 64 });
    assert_eq!(
        error.to_string(),
        "JSON merge patch nesting exceeds the maximum depth of 64"
    );
}

#[test]
fn a_non_object_leaf_at_the_depth_limit_is_not_an_error() {
    let patch = nested_object_patch(64, nested_array(100));
    let result = apply_json_merge_patch(&json("{}"), &patch).unwrap();
    assert_eq!(result, patch);
}

#[test]
fn a_deep_array_patch_replaces_the_target_whole() {
    let patch = nested_array(100);
    let result = apply_json_merge_patch(&json(r#"{"a":1}"#), &patch).unwrap();
    assert_eq!(result, patch);
}
