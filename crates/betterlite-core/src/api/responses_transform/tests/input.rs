use betterlite_models::controls::json_value::JsonValue;
use rstest::rstest;

use super::{build_request, value};
use crate::api::responses_transform::roles::is_history_item;

fn built_input(source: &str) -> JsonValue {
    build_request("m", Some(&value(source)), "{}", "{}")
        .input
        .expect("the input should be built")
}

#[test]
fn string_input_passes_through() {
    assert_eq!(built_input("\"hello\""), value("\"hello\""));
}

#[test]
fn step_items_map_to_user_and_assistant_messages() {
    assert_eq!(
        built_input(
            r#"[
                {"type": "user_input", "content": "hi"},
                {"type": "model_output", "content": [{"type": "text", "text": "yo"}]}
            ]"#
        ),
        value(
            r#"[
                {"role": "user", "content": [{"type": "input_text", "text": "hi"}]},
                {"role": "assistant", "content": [{"type": "output_text", "text": "yo"}]}
            ]"#
        )
    );
}

#[test]
fn turn_objects_map_their_roles() {
    assert_eq!(
        built_input(
            r#"[
                {"role": "user", "content": "hi"},
                {"role": "model", "content": [{"type": "text", "text": "yo"}]}
            ]"#
        ),
        value(
            r#"[
                {"role": "user", "content": [{"type": "input_text", "text": "hi"}]},
                {"role": "assistant", "content": [{"type": "output_text", "text": "yo"}]}
            ]"#
        )
    );
}

#[test]
fn a_missing_or_empty_role_defaults_to_user() {
    assert_eq!(
        built_input(r#"[{"role": null, "content": []}, {"role": "", "content": "x"}]"#),
        value(
            r#"[
                {"role": "user", "content": []},
                {"role": "user", "content": [{"type": "input_text", "text": "x"}]}
            ]"#
        )
    );
}

#[test]
fn another_turn_role_is_kept_as_given() {
    assert_eq!(
        built_input(r#"[{"role": "system", "content": "s"}]"#),
        value(r#"[{"role": "system", "content": [{"type": "input_text", "text": "s"}]}]"#)
    );
}

#[test]
fn the_step_type_wins_over_the_role() {
    assert_eq!(
        built_input(r#"[{"type": "model_output", "role": "user", "content": "x"}]"#),
        value(r#"[{"role": "assistant", "content": [{"type": "output_text", "text": "x"}]}]"#)
    );
}

#[test]
fn a_text_item_is_rebuilt_without_its_other_keys() {
    assert_eq!(
        built_input(r#"[{"type": "text", "text": "z", "annotations": []}]"#),
        value(r#"[{"role": "user", "content": [{"type": "input_text", "text": "z"}]}]"#)
    );
}

#[test]
fn a_turn_without_leaves_an_empty_content_list() {
    assert_eq!(
        built_input(r#"[{"role": "user"}]"#),
        value(r#"[{"role": "user", "content": []}]"#)
    );
    assert_eq!(
        built_input(r#"[{"role": "user", "content": null}]"#),
        value(r#"[{"role": "user", "content": []}]"#)
    );
}

#[test]
fn a_content_list_without_history_items_wraps_as_one_user_message() {
    assert_eq!(
        built_input(r#"["a", {"type": "image", "data": "d"}]"#),
        value(
            r#"[{"role": "user", "content": [
                {"type": "input_text", "text": "a"},
                {"type": "image", "data": "d"}
            ]}]"#
        )
    );
}

#[test]
fn an_empty_list_wraps_as_an_empty_user_message() {
    assert_eq!(
        built_input("[]"),
        value(r#"[{"role": "user", "content": []}]"#)
    );
}

#[test]
fn a_mixed_list_keeps_only_the_history_items() {
    assert_eq!(
        built_input(r#"["dropped", {"role": "user", "content": "kept"}]"#),
        value(r#"[{"role": "user", "content": [{"type": "input_text", "text": "kept"}]}]"#)
    );
}

#[test]
fn a_content_entry_with_list_content_becomes_a_user_message() {
    assert_eq!(
        built_input(
            r#"{"content": [{"type": "text", "text": "z"}, {"type": "image", "data": "d"}]}"#
        ),
        value(
            r#"[{"role": "user", "content": [
                {"type": "input_text", "text": "z"},
                {"type": "image", "data": "d"}
            ]}]"#
        )
    );
}

#[test]
fn a_content_entry_without_list_content_keeps_the_whole_object() {
    assert_eq!(
        built_input(r#"{"content": "z"}"#),
        value(r#"[{"role": "user", "content": [{"content": "z"}]}]"#)
    );
    assert_eq!(
        built_input(r#"{"content": null}"#),
        value(r#"[{"role": "user", "content": [{"content": null}]}]"#)
    );
}

#[test]
fn a_single_content_object_becomes_a_user_message() {
    assert_eq!(
        built_input(r#"{"type": "text", "text": "z"}"#),
        value(r#"[{"role": "user", "content": [{"type": "input_text", "text": "z"}]}]"#)
    );
}

#[test]
fn text_content_type_follows_the_role_and_other_items_pass_through() {
    assert_eq!(
        built_input(
            r#"[{"type": "model_output", "content": [
                {"type": "text", "text": "yo"},
                {"type": "input_image", "image_url": "u"}
            ]}]"#
        ),
        value(
            r#"[{"role": "assistant", "content": [
                {"type": "output_text", "text": "yo"},
                {"type": "input_image", "image_url": "u"}
            ]}]"#
        )
    );
}

#[test]
fn text_content_without_text_is_empty() {
    assert_eq!(
        built_input(r#"[{"type": "user_input", "content": [{"type": "text"}]}]"#),
        value(r#"[{"role": "user", "content": [{"type": "input_text", "text": ""}]}]"#)
    );
    assert_eq!(
        built_input(r#"[{"type": "user_input", "content": [{"type": "text", "text": null}]}]"#),
        value(r#"[{"role": "user", "content": [{"type": "input_text", "text": ""}]}]"#)
    );
}

#[rstest]
#[case::number("5", "input must be a string, an object or a list of them")]
#[case::boolean("true", "input must be a string, an object or a list of them")]
fn invalid_input_shapes_are_refused(#[case] source: &str, #[case] message: &str) {
    assert_eq!(super::request_error(source), message);
}

#[rstest]
#[case::number("[5]")]
#[case::boolean("[true]")]
#[case::null("[null]")]
fn invalid_content_items_are_refused(#[case] source: &str) {
    assert_eq!(
        super::request_error(source),
        "the input content item must be text or an object"
    );
}

#[test]
fn wrong_typed_content_text_is_refused() {
    assert_eq!(
        super::request_error(r#"[{"role": "user", "content": [{"type": "text", "text": 5}]}]"#),
        "content text must be a string"
    );
}

#[test]
fn a_wrong_typed_role_is_refused() {
    assert_eq!(
        super::request_error(r#"[{"role": 5, "content": []}]"#),
        "role must be a string"
    );
}

#[test]
fn history_items_are_objects_with_a_role_or_a_step_type() {
    assert!(is_history_item(&value(r#"{"role": "user"}"#)));
    assert!(is_history_item(&value(r#"{"role": null}"#)));
    assert!(is_history_item(&value(r#"{"type": "user_input"}"#)));
    assert!(is_history_item(&value(r#"{"type": "model_output"}"#)));
    assert!(!is_history_item(&value(r#"{"type": "text", "text": "x"}"#)));
    assert!(!is_history_item(&value(r#"{"type": "user_inputt"}"#)));
    assert!(!is_history_item(&value("\"hi\"")));
    assert!(!is_history_item(&value("[]")));
    assert!(!is_history_item(&value("5")));
}
