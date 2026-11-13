use rstest::rstest;

use super::support::json;
use crate::api::responses_parameters::error::ResponsesParametersError;
use crate::api::responses_parameters::prompt_merge::{
    NO_INPUT_MESSAGES_WARNING, NON_MESSAGE_ITEMS_WARNING, PromptMergeWarning, ReturnedMessage,
    input_to_chat_messages, merge_prompt_management_input, shape_prompt_managed_message,
};

fn returned(source: Option<usize>, message: &str) -> ReturnedMessage {
    ReturnedMessage {
        source,
        message: json(message),
    }
}

#[test]
fn null_input_has_no_messages() {
    assert!(input_to_chat_messages(None).expect("no input").is_empty());
    let null = json("null");
    assert!(
        input_to_chat_messages(Some(&null))
            .expect("null input")
            .is_empty()
    );
}

#[test]
fn a_string_input_becomes_a_user_message() {
    let input = json(r#""hi""#);
    let client = input_to_chat_messages(Some(&input)).expect("a string input");
    assert_eq!(client.len(), 1);
    assert_eq!(client[0].position, None);
    assert_eq!(
        client[0].message,
        json(r#"{"role": "user", "content": "hi"}"#)
    );
}

#[test]
fn a_list_input_keeps_only_objects_with_a_role() {
    let input = json(
        r#"[{"type": "function_call", "name": "f"}, {"role": "user", "content": "hi"}, {"type": "reasoning", "id": "r"}, {"role": "assistant", "content": "ok"}]"#,
    );
    let client = input_to_chat_messages(Some(&input)).expect("a list input");
    let positions: Vec<Option<usize>> = client.iter().map(|entry| entry.position).collect();
    assert_eq!(positions, [Some(1), Some(3)]);
    assert_eq!(
        client[0].message,
        json(r#"{"role": "user", "content": "hi"}"#)
    );
}

#[rstest]
#[case::boolean("true")]
#[case::number("1")]
#[case::object(r#"{"role": "user"}"#)]
fn a_wrong_input_type_is_an_error(#[case] source: &str) {
    let input = json(source);
    assert!(matches!(
        input_to_chat_messages(Some(&input)),
        Err(ResponsesParametersError::NotTextOrList { .. })
    ));
}

#[test]
fn a_text_part_becomes_input_text_for_a_non_assistant_message() {
    let message = json(
        r#"{"role": "user", "content": [{"type": "text", "text": "a", "annotations": []}, {"type": "image_url", "image_url": {"url": "u"}}]}"#,
    );
    assert_eq!(
        shape_prompt_managed_message(&message),
        json(
            r#"{"role": "user", "content": [{"type": "input_text", "text": "a", "annotations": []}, {"type": "image_url", "image_url": {"url": "u"}}]}"#
        )
    );
}

#[rstest]
#[case::assistant(r#"{"role": "assistant", "content": [{"type": "text", "text": "a"}]}"#)]
#[case::non_list_content(r#"{"role": "user", "content": "a"}"#)]
#[case::other_parts(r#"{"role": "user", "content": [{"type": "input_text", "text": "a"}]}"#)]
#[case::not_an_object(r#""hi""#)]
fn non_matching_messages_are_returned_unchanged(#[case] source: &str) {
    let message = json(source);
    assert_eq!(shape_prompt_managed_message(&message), message);
}

#[test]
fn all_message_items_return_the_shaped_merged_messages() {
    let original = json(r#"[{"role": "user", "content": "hi"}]"#);
    let merged = [returned(Some(0), r#"{"role": "user", "content": "hi"}"#)];
    let outcome = merge_prompt_management_input(&original, &merged).expect("merges");
    assert_eq!(
        outcome.input,
        json(r#"[{"role": "user", "content": "hi"}]"#)
    );
    assert_eq!(outcome.warning, None);
}

#[test]
fn no_message_slots_warn_and_keep_the_original_input() {
    let original = json(r#"[{"type": "function_call", "name": "f"}]"#);
    let merged = [returned(None, r#"{"role": "user", "content": "hi"}"#)];
    let outcome = merge_prompt_management_input(&original, &merged).expect("merges");
    assert_eq!(outcome.input, original);
    assert_eq!(outcome.warning, Some(PromptMergeWarning::NoInputMessages));
    assert_eq!(
        PromptMergeWarning::NoInputMessages.text(),
        NO_INPUT_MESSAGES_WARNING
    );
}

#[test]
fn corresponding_messages_replace_their_slots() {
    let original = json(
        r#"[{"type": "function_call", "name": "f"}, {"role": "user", "content": "hi", "id": "u"}, {"type": "reasoning", "id": "r"}, {"role": "assistant", "content": "ok", "id": "a"}]"#,
    );
    let merged = [
        returned(None, r#"{"role": "user", "content": "changed", "id": "u"}"#),
        returned(
            None,
            r#"{"role": "assistant", "content": "done", "id": "a"}"#,
        ),
    ];
    let outcome = merge_prompt_management_input(&original, &merged).expect("merges");
    assert_eq!(
        outcome.input,
        json(
            r#"[{"type": "function_call", "name": "f"}, {"role": "user", "content": "changed", "id": "u"}, {"type": "reasoning", "id": "r"}, {"role": "assistant", "content": "done", "id": "a"}]"#
        )
    );
    assert_eq!(outcome.warning, None);
}

#[test]
fn a_changed_id_breaks_correspondence() {
    let original = json(
        r#"[{"type": "function_call", "name": "f"}, {"role": "user", "content": "hi", "id": "u"}]"#,
    );
    let merged = [returned(
        None,
        r#"{"role": "user", "content": "hi", "id": "v"}"#,
    )];
    let outcome = merge_prompt_management_input(&original, &merged).expect("merges");
    assert_eq!(
        outcome.input,
        json(r#"[{"role": "user", "content": "hi", "id": "v"}]"#)
    );
    assert_eq!(
        outcome.warning,
        Some(PromptMergeWarning::NonMessageItemsDropped)
    );
    assert_eq!(
        PromptMergeWarning::NonMessageItemsDropped.text(),
        NON_MESSAGE_ITEMS_WARNING
    );
}

#[test]
fn a_changed_role_breaks_correspondence() {
    let original =
        json(r#"[{"type": "function_call", "name": "f"}, {"role": "user", "content": "hi"}]"#);
    let merged = [returned(None, r#"{"role": "assistant", "content": "hi"}"#)];
    let outcome = merge_prompt_management_input(&original, &merged).expect("merges");
    assert_eq!(
        outcome.input,
        json(r#"[{"role": "assistant", "content": "hi"}]"#)
    );
    assert_eq!(
        outcome.warning,
        Some(PromptMergeWarning::NonMessageItemsDropped)
    );
}

#[test]
fn an_absent_original_id_skips_the_id_comparison() {
    let original =
        json(r#"[{"type": "function_call", "name": "f"}, {"role": "user", "content": "hi"}]"#);
    let merged = [returned(
        None,
        r#"{"role": "user", "content": "hi", "id": "v"}"#,
    )];
    let outcome = merge_prompt_management_input(&original, &merged).expect("merges");
    assert_eq!(
        outcome.input,
        json(
            r#"[{"type": "function_call", "name": "f"}, {"role": "user", "content": "hi", "id": "v"}]"#
        )
    );
    assert_eq!(outcome.warning, None);
}

#[test]
fn preserved_messages_keep_their_prefixes_and_trailing_items() {
    let original = json(
        r#"[{"type": "function_call", "name": "f"}, {"role": "user", "content": "hi"}, {"type": "reasoning", "id": "r"}, {"role": "assistant", "content": "ok"}, {"type": "function_call_output", "output": "x"}]"#,
    );
    let merged = [
        returned(None, r#"{"role": "system", "content": "rules"}"#),
        returned(Some(0), r#"{"role": "user", "content": "hi"}"#),
        returned(Some(1), r#"{"role": "assistant", "content": "ok"}"#),
    ];
    let outcome = merge_prompt_management_input(&original, &merged).expect("merges");
    assert_eq!(
        outcome.input,
        json(
            r#"[{"role": "system", "content": "rules"}, {"type": "function_call", "name": "f"}, {"role": "user", "content": "hi"}, {"type": "reasoning", "id": "r"}, {"role": "assistant", "content": "ok"}, {"type": "function_call_output", "output": "x"}]"#
        )
    );
    assert_eq!(outcome.warning, None);
}

#[test]
fn equal_copies_do_not_take_the_identity_branch() {
    let original = json(
        r#"[{"type": "function_call", "name": "f"}, {"role": "user", "content": "hi"}, {"type": "reasoning", "id": "r"}, {"role": "assistant", "content": "ok"}, {"type": "function_call_output", "output": "x"}]"#,
    );
    let merged = [
        returned(None, r#"{"role": "system", "content": "rules"}"#),
        returned(None, r#"{"role": "user", "content": "hi"}"#),
        returned(None, r#"{"role": "assistant", "content": "ok"}"#),
    ];
    let outcome = merge_prompt_management_input(&original, &merged).expect("merges");
    assert_eq!(
        outcome.input,
        json(
            r#"[{"role": "system", "content": "rules"}, {"role": "user", "content": "hi"}, {"role": "assistant", "content": "ok"}]"#
        )
    );
    assert_eq!(
        outcome.warning,
        Some(PromptMergeWarning::NonMessageItemsDropped)
    );
}

#[test]
fn an_in_place_change_to_a_returned_message_counts_for_correspondence() {
    let original = json(
        r#"[{"role": "user", "content": "a"}, {"type": "function_call", "name": "f"}, {"role": "assistant", "content": "b"}]"#,
    );
    let merged = [
        returned(Some(1), r#"{"role": "user", "content": "b2"}"#),
        returned(Some(0), r#"{"role": "user", "content": "a"}"#),
    ];
    let outcome = merge_prompt_management_input(&original, &merged).expect("merges");
    assert_eq!(
        outcome.input,
        json(
            r#"[{"role": "user", "content": "b2"}, {"type": "function_call", "name": "f"}, {"role": "user", "content": "a"}]"#
        )
    );
    assert_eq!(outcome.warning, None);
}

#[test]
fn an_extra_merged_message_breaks_correspondence() {
    let original = json(
        r#"[{"type": "function_call", "name": "f"}, {"role": "user", "content": "a"}, {"role": "assistant", "content": "b"}]"#,
    );
    let merged = [
        returned(None, r#"{"role": "user", "content": "a"}"#),
        returned(None, r#"{"role": "assistant", "content": "b"}"#),
        returned(None, r#"{"role": "user", "content": "c"}"#),
    ];
    let outcome = merge_prompt_management_input(&original, &merged).expect("merges");
    assert_eq!(
        outcome.input,
        json(
            r#"[{"role": "user", "content": "a"}, {"role": "assistant", "content": "b"}, {"role": "user", "content": "c"}]"#
        )
    );
    assert_eq!(
        outcome.warning,
        Some(PromptMergeWarning::NonMessageItemsDropped)
    );
}

#[test]
fn an_empty_original_list_returns_the_shaped_merged_messages() {
    let original = json("[]");
    let merged = [
        returned(None, r#"{"role": "user", "content": "hi"}"#),
        returned(None, r#"{"role": "assistant", "content": "ok"}"#),
    ];
    let outcome = merge_prompt_management_input(&original, &merged).expect("merges");
    assert_eq!(
        outcome.input,
        json(r#"[{"role": "user", "content": "hi"}, {"role": "assistant", "content": "ok"}]"#)
    );
    assert_eq!(outcome.warning, None);
}

#[test]
fn a_string_original_returns_the_shaped_merged_messages() {
    let original = json(r#""hi""#);
    let merged = [returned(
        None,
        r#"{"role": "user", "content": [{"type": "text", "text": "a"}]}"#,
    )];
    let outcome = merge_prompt_management_input(&original, &merged).expect("merges");
    assert_eq!(
        outcome.input,
        json(r#"[{"role": "user", "content": [{"type": "input_text", "text": "a"}]}]"#)
    );
    assert_eq!(outcome.warning, None);
}

#[test]
fn a_source_outside_the_client_messages_is_an_error() {
    let original = json(r#"[{"type": "function_call", "name": "f"}]"#);
    let merged = [returned(Some(4), r#"{"role": "user", "content": "hi"}"#)];
    assert!(matches!(
        merge_prompt_management_input(&original, &merged),
        Err(ResponsesParametersError::MessageSource { index: 4 })
    ));
}

#[test]
fn a_non_text_input_to_merge_is_an_error() {
    let original = json("null");
    assert!(matches!(
        merge_prompt_management_input(&original, &[]),
        Err(ResponsesParametersError::NotTextOrList { .. })
    ));
}
