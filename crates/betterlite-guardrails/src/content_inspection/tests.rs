#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::{
    InspectionMessage, NON_CONVERSATIONAL_CALL_TYPES, TEXT_CONTENT_CALL_TYPES, TEXT_PART_TYPES,
    apply_redacted_messages_back, build_inspection_messages, has_non_string_content,
    is_string_batch_input, iter_message_text,
};

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn list(items: &[GuardrailValue]) -> GuardrailValue {
    GuardrailValue::Array(items.to_vec())
}

fn body(entries: &[(&str, GuardrailValue)]) -> BTreeMap<String, GuardrailValue> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn object(entries: &[(&str, GuardrailValue)]) -> GuardrailValue {
    GuardrailValue::Object(body(entries))
}

fn message(role: &str, content: &str) -> InspectionMessage {
    InspectionMessage {
        role: role.to_owned(),
        content: content.to_owned(),
    }
}

#[test]
fn call_type_text_constants_hold_the_conversational_names() {
    assert_eq!(
        TEXT_CONTENT_CALL_TYPES,
        ["completion", "acompletion", "aresponses"]
    );
}

#[test]
fn call_type_non_conversational_constants_hold_the_embedding_names() {
    assert_eq!(NON_CONVERSATIONAL_CALL_TYPES, ["embedding", "aembedding"]);
}

#[test]
fn call_type_text_part_constants_hold_the_five_part_names() {
    assert_eq!(
        TEXT_PART_TYPES,
        [
            "text",
            "input_text",
            "output_text",
            "summary_text",
            "reasoning_text"
        ]
    );
}

#[rstest]
#[case::responses("responses")]
#[case::anthropic_messages("anthropic_messages")]
#[case::conversation("conversation")]
fn call_type_names_outside_the_lists_are_in_neither(#[case] call_type: &str) {
    assert!(!TEXT_CONTENT_CALL_TYPES.contains(&call_type));
    assert!(!NON_CONVERSATIONAL_CALL_TYPES.contains(&call_type));
}

#[test]
fn text_reads_messages_before_input_in_order() {
    let data = body(&[
        (
            "messages",
            list(&[
                object(&[("role", text("user")), ("content", text("first"))]),
                object(&[
                    ("role", text("assistant")),
                    (
                        "content",
                        list(&[
                            object(&[("type", text("text")), ("text", text("second"))]),
                            text("third"),
                            object(&[("type", text("image_url")), ("text", text("fourth"))]),
                            object(&[
                                ("type", text("image_url")),
                                ("image_url", object(&[("url", text("https://host/x.png"))])),
                            ]),
                        ]),
                    ),
                ]),
                text("not an object"),
            ]),
        ),
        (
            "input",
            list(&[text("fifth"), object(&[("text", text("sixth"))])]),
        ),
    ]);
    assert_eq!(
        iter_message_text(&data),
        ["first", "second", "third", "fourth", "fifth", "sixth"]
    );
}

#[test]
fn text_skips_empty_strings_and_empty_part_text() {
    let data = body(&[(
        "messages",
        list(&[
            object(&[("content", text(""))]),
            object(&[(
                "content",
                list(&[
                    text(""),
                    object(&[("text", text(""))]),
                    object(&[("text", text("kept"))]),
                ]),
            )]),
        ]),
    )]);
    assert_eq!(iter_message_text(&data), ["kept"]);
}

#[test]
fn text_skips_parts_that_are_not_strings_or_objects() {
    let data = body(&[(
        "messages",
        list(&[object(&[(
            "content",
            list(&[
                text(""),
                object(&[("text", text(""))]),
                GuardrailValue::SignedInteger(5),
                GuardrailValue::Null,
            ]),
        )])]),
    )]);
    assert!(iter_message_text(&data).is_empty());
}

#[test]
fn text_ignores_content_that_is_not_a_string_or_list() {
    let data = body(&[(
        "messages",
        list(&[object(&[
            ("role", text("user")),
            (
                "content",
                object(&[("type", text("text")), ("text", text("nested"))]),
            ),
        ])]),
    )]);
    assert!(iter_message_text(&data).is_empty());
}

#[test]
fn text_reads_a_string_input_as_one_message() {
    let data = body(&[("input", text("hello"))]);
    assert_eq!(iter_message_text(&data), ["hello"]);
}

#[test]
fn text_ignores_an_input_that_is_not_a_list_or_string() {
    let integer = body(&[("input", GuardrailValue::SignedInteger(5))]);
    let part_text = body(&[("input", object(&[("text", text("x"))]))]);
    let reasoning = body(&[(
        "input",
        object(&[("type", text("reasoning")), ("content", text("x"))]),
    )]);
    assert!(iter_message_text(&integer).is_empty());
    assert!(iter_message_text(&part_text).is_empty());
    assert!(iter_message_text(&reasoning).is_empty());
}

#[test]
fn input_item_string_becomes_a_user_message() {
    let data = body(&[("input", list(&[text("hello"), text("")]))]);
    assert_eq!(build_inspection_messages(&data), [message("user", "hello")]);
}

#[test]
fn input_item_part_text_uses_the_item_role_or_user() {
    let with_role = body(&[(
        "input",
        list(&[object(&[
            ("type", text("input_text")),
            ("text", text("hi")),
            ("role", text("system")),
        ])]),
    )]);
    let without_role = body(&[(
        "input",
        list(&[object(&[
            ("type", text("input_text")),
            ("text", text("hi")),
        ])]),
    )]);
    assert_eq!(
        build_inspection_messages(&with_role),
        [message("system", "hi")]
    );
    assert_eq!(
        build_inspection_messages(&without_role),
        [message("user", "hi")]
    );
}

#[test]
fn input_item_part_text_wins_over_a_content_key() {
    let data = body(&[(
        "input",
        list(&[object(&[
            ("type", text("message")),
            ("text", text("top")),
            ("content", text("ignored")),
        ])]),
    )]);
    assert_eq!(build_inspection_messages(&data), [message("user", "top")]);
    assert_eq!(iter_message_text(&data), ["top"]);
}

#[test]
fn input_item_reasoning_yields_content_then_summary() {
    let data = body(&[(
        "input",
        list(&[object(&[
            ("type", text("reasoning")),
            (
                "content",
                list(&[object(&[
                    ("type", text("reasoning_text")),
                    ("text", text("thought")),
                ])]),
            ),
            (
                "summary",
                list(&[object(&[
                    ("type", text("summary_text")),
                    ("text", text("short")),
                ])]),
            ),
        ])]),
    )]);
    assert_eq!(
        build_inspection_messages(&data),
        [
            message("assistant", "thought"),
            message("assistant", "short")
        ]
    );
    assert_eq!(iter_message_text(&data), ["thought", "short"]);
}

#[test]
fn input_item_reasoning_without_content_or_list_summary_yields_nothing() {
    let bare = body(&[("input", list(&[object(&[("type", text("reasoning"))])]))]);
    let string_summary = body(&[(
        "input",
        list(&[object(&[
            ("type", text("reasoning")),
            ("summary", text("plain")),
        ])]),
    )]);
    assert!(build_inspection_messages(&bare).is_empty());
    assert!(iter_message_text(&bare).is_empty());
    assert!(build_inspection_messages(&string_summary).is_empty());
}

#[test]
fn input_item_content_key_uses_the_item_role_or_user() {
    let plain = body(&[("input", list(&[object(&[("content", text("body"))])]))]);
    let listed = body(&[(
        "input",
        list(&[object(&[
            ("role", text("assistant")),
            (
                "content",
                list(&[object(&[("type", text("text")), ("text", text("body"))])]),
            ),
        ])]),
    )]);
    assert_eq!(build_inspection_messages(&plain), [message("user", "body")]);
    assert_eq!(
        build_inspection_messages(&listed),
        [message("assistant", "body")]
    );
}

#[rstest]
#[case::function_call_output("function_call_output", "result")]
#[case::custom_tool_call_output("custom_tool_call_output", "custom")]
fn input_item_output_items_default_to_role_tool(#[case] item_type: &str, #[case] output: &str) {
    let data = body(&[(
        "input",
        list(&[object(&[
            ("type", text(item_type)),
            ("output", text(output)),
        ])]),
    )]);
    assert_eq!(build_inspection_messages(&data), [message("tool", output)]);
}

#[test]
fn input_item_output_keeps_a_present_role() {
    let data = body(&[(
        "input",
        list(&[object(&[
            ("type", text("function_call_output")),
            ("output", text("result")),
            ("role", text("user")),
        ])]),
    )]);
    assert_eq!(
        build_inspection_messages(&data),
        [message("user", "result")]
    );
}

#[test]
fn input_item_other_objects_are_dropped() {
    let data = body(&[(
        "input",
        list(&[
            object(&[("type", text("function_call")), ("name", text("f"))]),
            object(&[("type", text("function_call_output"))]),
            GuardrailValue::SignedInteger(5),
            GuardrailValue::Null,
        ]),
    )]);
    assert!(build_inspection_messages(&data).is_empty());
    assert!(iter_message_text(&data).is_empty());
}

#[test]
fn inspection_joins_fragments_with_a_newline() {
    let data = body(&[(
        "messages",
        list(&[object(&[
            ("role", text("user")),
            (
                "content",
                list(&[text("one"), object(&[("text", text("two"))])]),
            ),
        ])]),
    )]);
    assert_eq!(
        build_inspection_messages(&data),
        [message("user", "one\ntwo")]
    );
}

#[test]
fn inspection_drops_messages_without_text() {
    let data = body(&[(
        "messages",
        list(&[
            object(&[("role", text("user")), ("content", list(&[]))]),
            object(&[("role", text("user")), ("content", text(""))]),
            object(&[("role", text("user"))]),
            object(&[("role", text("assistant")), ("content", text("kept"))]),
        ]),
    )]);
    assert_eq!(
        build_inspection_messages(&data),
        [message("assistant", "kept")]
    );
}

#[rstest]
#[case::missing(vec![("content", text("x"))], "user")]
#[case::null_role(vec![("role", GuardrailValue::Null), ("content", text("x"))], "user")]
#[case::empty_role(vec![("role", text("")), ("content", text("x"))], "user")]
#[case::named_role(vec![("role", text("system")), ("content", text("x"))], "system")]
fn inspection_falls_back_to_the_user_role(
    #[case] entries: Vec<(&str, GuardrailValue)>,
    #[case] role: &str,
) {
    let data = body(&[("messages", list(&[object(&entries)]))]);
    assert_eq!(build_inspection_messages(&data), [message(role, "x")]);
}

#[test]
fn inspection_drops_content_that_is_not_a_string_or_list() {
    let data = body(&[(
        "messages",
        list(&[object(&[
            ("role", text("user")),
            (
                "content",
                object(&[("type", text("text")), ("text", text("nested"))]),
            ),
        ])]),
    )]);
    assert!(build_inspection_messages(&data).is_empty());
}

#[test]
fn inspection_ignores_an_object_input() {
    let alone = body(&[("input", object(&[("text", text("x"))]))]);
    let with_messages = body(&[
        (
            "messages",
            list(&[object(&[("role", text("user")), ("content", text("chat"))])]),
        ),
        (
            "input",
            object(&[("type", text("reasoning")), ("content", text("x"))]),
        ),
    ]);
    assert!(build_inspection_messages(&alone).is_empty());
    assert_eq!(
        build_inspection_messages(&with_messages),
        [message("user", "chat")]
    );
}

#[test]
fn inspection_keeps_messages_before_input() {
    let data = body(&[
        (
            "messages",
            list(&[object(&[("role", text("user")), ("content", text("chat"))])]),
        ),
        ("input", list(&[text("resp")])),
    ]);
    assert_eq!(
        build_inspection_messages(&data),
        [message("user", "chat"), message("user", "resp")]
    );
}

#[rstest]
#[case::null_messages(vec![("messages", GuardrailValue::Null), ("input", list(&[text("a")]))])]
#[case::empty_messages(vec![("messages", list(&[])), ("input", list(&[text("a")]))])]
#[case::empty_input(vec![("input", list(&[]))])]
#[case::mixed_input(vec![("input", list(&[text("a"), object(&[("text", text("b"))])]))])]
#[case::number_input(vec![("input", list(&[GuardrailValue::SignedInteger(1)]))])]
#[case::string_input(vec![("input", text("a"))])]
fn batch_rejects_a_shape_that_is_not_a_string_list(#[case] entries: Vec<(&str, GuardrailValue)>) {
    assert!(!is_string_batch_input(&body(&entries)));
}

#[rstest]
#[case::plain(vec![text("a"), text("b")])]
#[case::with_empty(vec![text("a"), text(""), text("c")])]
fn batch_accepts_a_non_empty_list_of_strings(#[case] items: Vec<GuardrailValue>) {
    let data = body(&[("input", list(&items))]);
    assert!(is_string_batch_input(&data));
}

#[test]
fn write_back_replaces_only_non_empty_batch_positions() {
    let mut data = body(&[("input", list(&[text("a"), text(""), text("c")]))]);
    let redacted = [
        object(&[("content", text("A"))]),
        object(&[("content", text("C"))]),
    ];
    assert!(apply_redacted_messages_back(&mut data, &redacted));
    assert_eq!(
        data.get("input"),
        Some(&list(&[text("A"), text(""), text("C")]))
    );
}

#[test]
fn write_back_joins_content_fragments_in_a_batch() {
    let mut data = body(&[("input", list(&[text("a"), text("b")]))]);
    let redacted = [
        object(&[(
            "content",
            list(&[text("one"), object(&[("text", text("two"))])]),
        )]),
        object(&[("content", text("B"))]),
    ];
    assert!(apply_redacted_messages_back(&mut data, &redacted));
    assert_eq!(
        data.get("input"),
        Some(&list(&[text("one\ntwo"), text("B")]))
    );
}

#[test]
fn write_back_accepts_an_empty_batch_of_empty_strings() {
    let mut data = body(&[("input", list(&[text(""), text("")]))]);
    assert!(apply_redacted_messages_back(&mut data, &[]));
    assert_eq!(data.get("input"), Some(&list(&[text(""), text("")])));
}

#[rstest]
#[case::count_mismatch(vec![text("a"), text("b")], vec![object(&[("content", text("A"))])])]
#[case::entry_without_content(vec![text("a")], vec![object(&[("role", text("user"))])])]
#[case::entry_null_content(vec![text("a")], vec![object(&[("content", GuardrailValue::Null)])])]
#[case::entry_not_an_object(vec![text("a")], vec![text("A")])]
fn write_back_rejects_a_batch_answer_it_cannot_place(
    #[case] batch: Vec<GuardrailValue>,
    #[case] redacted: Vec<GuardrailValue>,
) {
    let mut data = body(&[("input", list(&batch))]);
    let original = data.clone();
    assert!(!apply_redacted_messages_back(&mut data, &redacted));
    assert_eq!(data, original);
}

#[test]
fn write_back_replaces_messages_outside_a_batch() {
    let mut data = body(&[(
        "messages",
        list(&[object(&[("role", text("user")), ("content", text("orig"))])]),
    )]);
    let redacted = vec![object(&[("role", text("user")), ("content", text("new"))])];
    assert!(apply_redacted_messages_back(&mut data, &redacted));
    assert_eq!(data.get("messages"), Some(&GuardrailValue::Array(redacted)));
}

#[test]
fn write_back_joins_all_fragments_into_a_string_input() {
    let mut data = body(&[("input", text("orig"))]);
    let redacted = vec![
        object(&[(
            "content",
            list(&[text("one"), object(&[("text", text("two"))])]),
        )]),
        text("bare"),
        object(&[("content", text("three"))]),
    ];
    assert!(apply_redacted_messages_back(&mut data, &redacted));
    assert_eq!(data.get("input"), Some(&text("one\ntwo\nthree")));
}

#[test]
fn write_back_updates_messages_and_input_together() {
    let mut data = body(&[
        (
            "messages",
            list(&[object(&[("role", text("user")), ("content", text("orig"))])]),
        ),
        ("input", text("orig")),
    ]);
    let redacted = vec![object(&[("role", text("user")), ("content", text("new"))])];
    assert!(apply_redacted_messages_back(&mut data, &redacted));
    assert_eq!(data.get("messages"), Some(&GuardrailValue::Array(redacted)));
    assert_eq!(data.get("input"), Some(&text("new")));
}

#[test]
fn write_back_leaves_a_list_input_unchanged() {
    let mut data = body(&[(
        "input",
        list(&[object(&[("type", text("input_text")), ("text", text("x"))])]),
    )]);
    let original = data.clone();
    assert!(apply_redacted_messages_back(
        &mut data,
        &[object(&[("content", text("y"))])]
    ));
    assert_eq!(data, original);
}

#[test]
fn write_back_empties_a_batch_position_for_object_content() {
    let mut data = body(&[("input", list(&[text("a"), text("b")]))]);
    let redacted = [
        object(&[("content", object(&[("text", text("x"))]))]),
        object(&[("content", text("B"))]),
    ];
    assert!(apply_redacted_messages_back(&mut data, &redacted));
    assert_eq!(data.get("input"), Some(&list(&[text(""), text("B")])));
}

#[test]
fn write_back_empties_a_string_input_for_object_content() {
    let mut data = body(&[("input", text("orig"))]);
    assert!(apply_redacted_messages_back(
        &mut data,
        &[object(&[("content", object(&[("text", text("x"))]))])]
    ));
    assert_eq!(data.get("input"), Some(&text("")));
}

#[test]
fn write_back_treats_a_null_messages_key_as_outside_a_batch() {
    let mut data = body(&[
        ("messages", GuardrailValue::Null),
        ("input", list(&[text("a"), text("b")])),
    ]);
    let redacted = vec![object(&[("content", text("A"))])];
    assert!(apply_redacted_messages_back(&mut data, &redacted));
    assert_eq!(data.get("messages"), Some(&GuardrailValue::Array(redacted)));
    assert_eq!(data.get("input"), Some(&list(&[text("a"), text("b")])));
}

#[test]
fn non_string_content_is_true_for_list_content() {
    let data = body(&[(
        "messages",
        list(&[object(&[(
            "content",
            list(&[object(&[("text", text("x"))])]),
        )])]),
    )]);
    assert!(has_non_string_content(&data));
}

#[rstest]
#[case::empty_list(list(&[]))]
#[case::part_list(list(&[text("x")]))]
#[case::number(GuardrailValue::SignedInteger(5))]
#[case::object(object(&[("role", text("user"))]))]
fn non_string_content_is_true_for_a_non_string_input(#[case] input: GuardrailValue) {
    assert!(has_non_string_content(&body(&[("input", input)])));
}

#[test]
fn non_string_content_is_false_for_string_or_null_content() {
    let string_content = body(&[("messages", list(&[object(&[("content", text("x"))])]))]);
    let null_content = body(&[(
        "messages",
        list(&[object(&[("content", GuardrailValue::Null)])]),
    )]);
    let missing_content = body(&[("messages", list(&[object(&[("role", text("user"))])]))]);
    assert!(!has_non_string_content(&string_content));
    assert!(!has_non_string_content(&null_content));
    assert!(!has_non_string_content(&missing_content));
}

#[rstest]
#[case::string(text("x"))]
#[case::null(GuardrailValue::Null)]
fn non_string_content_is_false_for_a_string_or_null_input(#[case] input: GuardrailValue) {
    assert!(!has_non_string_content(&body(&[("input", input)])));
}

#[test]
fn non_string_content_is_false_for_an_empty_body() {
    assert!(!has_non_string_content(&body(&[])));
}

#[test]
fn non_string_content_ignores_non_object_messages() {
    let data = body(&[(
        "messages",
        list(&[text("x"), GuardrailValue::SignedInteger(1)]),
    )]);
    assert!(!has_non_string_content(&data));
}
