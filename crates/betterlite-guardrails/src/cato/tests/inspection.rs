use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::{body, field_text, list, message_pairs, object, text};
use crate::cato::inspection::{ExtraField, extra_inspection_sources, inspection_messages};
use crate::cato::schema_strings::{schema_string_messages, schema_string_refs};

#[test]
fn cato_guardrail_keeps_chat_messages_one_to_one_by_index() {
    let data = body(&[(
        "messages",
        list(&[
            object(&[("content", text("first")), ("role", text("user"))]),
            object(&[
                ("content", text("second")),
                ("name", text("tool")),
                ("role", text("assistant")),
            ]),
            text("not an object"),
        ]),
    )]);
    let messages = inspection_messages(&data);
    assert_eq!(messages.len(), 3);
    assert_eq!(field_text(&messages, "role"), ["user", "assistant", ""]);
    assert_eq!(field_text(&messages, "content"), ["first", "second", ""]);
    assert_eq!(
        messages.get(1),
        Some(&object(&[
            ("content", text("second")),
            ("name", text("tool")),
            ("role", text("assistant")),
        ]))
    );
    assert_eq!(messages.get(2), Some(&text("not an object")));
}

#[rstest]
#[case::text_parts(
    list(&[
        object(&[("type", text("text")), ("text", text("one"))]),
        object(&[("type", text("image_url")), ("image_url", object(&[("url", text("https://host/x.png"))]))]),
        text("two"),
        object(&[("type", text("text")), ("text", text(""))]),
    ]),
    "one\ntwo"
)]
#[case::no_text_parts(
    list(&[object(&[("type", text("image_url")), ("image_url", object(&[("url", text("https://host/x.png"))]))])]),
    ""
)]
#[case::empty_list(list(&[]), "")]
fn cato_guardrail_flattens_multimodal_content_into_text(
    #[case] content: GuardrailValue,
    #[case] expected: &str,
) {
    let data = body(&[(
        "messages",
        list(&[object(&[("content", content), ("role", text("user"))])]),
    )]);
    let messages = inspection_messages(&data);
    assert_eq!(field_text(&messages, "content"), [expected]);
    assert_eq!(field_text(&messages, "role"), ["user"]);
}

#[test]
fn cato_guardrail_appends_extra_sources_in_a_fixed_order() {
    let data = body(&[
        ("messages", list(&[object(&[("content", text("chat"))])])),
        ("input", text("input text")),
        ("instructions", text("be careful")),
        ("prompt", text("legacy prompt")),
        (
            "tools",
            list(&[object(&[(
                "function",
                object(&[("description", text("tool text"))]),
            )])]),
        ),
    ]);
    let messages = inspection_messages(&data);
    assert_eq!(
        message_pairs(&messages),
        [
            ("".to_owned(), "chat".to_owned()),
            ("user".to_owned(), "input text".to_owned()),
            ("system".to_owned(), "be careful".to_owned()),
            ("user".to_owned(), "legacy prompt".to_owned()),
            ("system".to_owned(), "tool text".to_owned()),
        ]
    );
    let fields = extra_inspection_sources(&data)
        .into_iter()
        .map(|source| source.field)
        .collect::<Vec<_>>();
    assert_eq!(
        fields,
        [
            ExtraField::Input,
            ExtraField::Instructions,
            ExtraField::Prompt,
            ExtraField::SchemaStrings
        ]
    );
}

#[rstest]
#[case::prompt_string("prompt", text("hello"), vec![("user", "hello")])]
#[case::prompt_empty_string("prompt", text(""), vec![])]
#[case::prompt_list(
    "prompt",
    list(&[
        text("one"),
        text(""),
        GuardrailValue::SignedInteger(2),
        object(&[("text", text("dropped"))]),
        text("three"),
    ]),
    vec![("user", "one"), ("user", "three")]
)]
#[case::input_string("input", text("hello"), vec![("user", "hello")])]
#[case::input_list(
    "input",
    list(&[
        text("first"),
        object(&[("role", text("assistant")), ("content", text("second"))]),
        object(&[("type", text("input_text")), ("text", text("third"))]),
        object(&[
            ("type", text("function_call_output")),
            ("output", text("fourth")),
        ]),
    ]),
    vec![("user", "first"), ("assistant", "second"), ("user", "third"), ("tool", "fourth")]
)]
fn cato_guardrail_appends_prompt_and_responses_input_messages(
    #[case] field: &str,
    #[case] value: GuardrailValue,
    #[case] expected: Vec<(&str, &str)>,
) {
    let messages = inspection_messages(&body(&[(field, value)]));
    let expected = expected
        .into_iter()
        .map(|(role, content)| (role.to_owned(), content.to_owned()))
        .collect::<Vec<_>>();
    assert_eq!(message_pairs(&messages), expected);
}

#[test]
fn cato_guardrail_appends_an_instruction_message_only_for_non_empty_text() {
    let empty = body(&[("instructions", text(""))]);
    assert!(inspection_messages(&empty).is_empty());
    let instruction = body(&[("instructions", text("be careful"))]);
    assert_eq!(
        message_pairs(&inspection_messages(&instruction)),
        [("system".to_owned(), "be careful".to_owned())]
    );
}

#[test]
fn cato_guardrail_finds_schema_strings_recursively_in_a_fixed_order() {
    let data = body(&[
        (
            "tools",
            list(&[object(&[(
                "function",
                object(&[
                    ("description", text("tool description")),
                    ("name", text("tool")),
                    (
                        "parameters",
                        object(&[
                            (
                                "properties",
                                object(&[(
                                    "q",
                                    object(&[
                                        ("description", text("query description")),
                                        ("enum", list(&[text("one"), text(""), text("two")])),
                                        ("examples", list(&[text("example")])),
                                    ]),
                                )]),
                            ),
                            ("title", text("params title")),
                        ]),
                    ),
                ]),
            )])]),
        ),
        (
            "functions",
            list(&[object(&[("description", text("function description"))])]),
        ),
        (
            "response_format",
            object(&[(
                "json_schema",
                object(&[(
                    "schema",
                    object(&[
                        ("title", text("response title")),
                        (
                            "properties",
                            object(&[(
                                "a",
                                object(&[("const", text("fixed")), ("default", text("fallback"))]),
                            )]),
                        ),
                    ]),
                )]),
            )]),
        ),
    ]);
    assert_eq!(
        field_text(&schema_string_messages(&data), "role"),
        [
            "tool description",
            "params title",
            "query description",
            "one",
            "two",
            "example",
            "function description",
            "response title",
            "fixed",
            "fallback"
        ]
    );
    assert_eq!(
        field_text(&schema_string_messages(&data), "role"),
        vec!["system".to_owned(); 10]
    );
}

#[test]
fn cato_guardrail_descends_into_nested_schema_arrays() {
    let data = body(&[(
        "tools",
        list(&[object(&[(
            "function",
            object(&[(
                "parameters",
                object(&[(
                    "anyOf",
                    list(&[
                        object(&[("description", text("branch one"))]),
                        object(&[("title", text("branch two"))]),
                    ]),
                )]),
            )]),
        )])]),
    )]);
    assert_eq!(
        field_text(&schema_string_messages(&data), "role"),
        ["branch one", "branch two"]
    );
}

#[test]
fn cato_guardrail_skips_empty_schema_strings_and_non_schema_roots() {
    let data = body(&[
        (
            "tools",
            list(&[
                object(&[("function", object(&[("description", text(""))]))]),
                object(&[("type", text("function"))]),
                text("not a tool"),
            ]),
        ),
        ("functions", list(&[text("not a function")])),
        ("response_format", object(&[("description", text(""))])),
    ]);
    assert!(schema_string_messages(&data).is_empty());
    assert!(schema_string_refs(&data).is_empty());
}

#[test]
fn cato_guardrail_reads_no_schema_strings_without_schema_fields() {
    let data = body(&[("prompt", text("plain"))]);
    assert!(schema_string_messages(&data).is_empty());
}

#[test]
fn cato_guardrail_ignores_an_empty_messages_list() {
    let data = body(&[("messages", list(&[]))]);
    assert!(inspection_messages(&data).is_empty());
}
