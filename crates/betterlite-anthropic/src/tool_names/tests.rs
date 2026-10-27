#![cfg(test)]

use std::collections::BTreeMap;

use super::{
    FunctionRecord, MessageRecord, RequestParams, ToolCallRecord, ToolChoiceRecord, ToolRecord,
    basic_sanitize_tool_name, build_tool_name_maps, rewrite_tool_names_in_messages,
    sanitize_tool_names_in_request,
};
use rstest::rstest;

#[rstest]
#[case::invalid_tool_name_characters(
    vec![String::from("weather/city"), String::from("a\u{1f600}b"), String::new()],
    BTreeMap::from([
        (String::from("weather_city"), String::from("weather/city")),
        (String::from("a_b"), String::from("a\u{1f600}b")),
    ])
)]
#[case::name_collision_suffix(
    vec![String::from("foo/bar"), String::from("foo_bar"), String::from("foo?bar")],
    BTreeMap::from([
        (String::from("foo_bar_2"), String::from("foo/bar")),
        (String::from("foo_bar_3"), String::from("foo?bar")),
    ])
)]
fn tool_names_map_cases(#[case] names: Vec<String>, #[case] expected: BTreeMap<String, String>) {
    assert_eq!(build_tool_name_maps(&names).reverse, expected);
}

#[rstest]
#[case::invalid_tool_name_characters("invalid_tool_name_characters")]
#[case::name_collision_suffix("name_collision_suffix")]
#[case::reverse_name_map("reverse_name_map")]
#[case::tool_choice_rewrite("tool_choice_rewrite")]
#[case::tool_message_rewrite("tool_message_rewrite")]
fn tool_names_cases(#[case] case: &str) {
    match case {
        "invalid_tool_name_characters" => tool_names_map_cases(
            vec![
                String::from("weather/city"),
                String::from("a\u{1f600}b"),
                String::new(),
            ],
            BTreeMap::from([
                (String::from("weather_city"), String::from("weather/city")),
                (String::from("a_b"), String::from("a\u{1f600}b")),
            ]),
        ),
        "name_collision_suffix" => tool_names_map_cases(
            vec![
                String::from("foo/bar"),
                String::from("foo_bar"),
                String::from("foo?bar"),
            ],
            BTreeMap::from([
                (String::from("foo_bar_2"), String::from("foo/bar")),
                (String::from("foo_bar_3"), String::from("foo?bar")),
            ]),
        ),
        "reverse_name_map" => reverse_name_map_contains_changed_entries_only(),
        "tool_choice_rewrite" => tool_choice_rewrite_copies_only_custom_tools(),
        "tool_message_rewrite" => tool_message_rewrite_preserves_unrelated_entries(),
        _ => (),
    }
}

#[test]
fn reverse_name_map_contains_changed_entries_only() {
    let names = [String::from("foo_bar"), String::from("foo/bar")];
    let maps = build_tool_name_maps(&names);
    assert_eq!(maps.forward["foo/bar"], "foo_bar_2");
    assert_eq!(maps.reverse["foo_bar_2"], "foo/bar");
}

#[test]
fn valid_name_characters_are_not_sanitized() {
    let names = [
        String::from("alpha_numeric9"),
        String::from("alpha-numeric9"),
    ];
    let maps = build_tool_name_maps(&names);
    assert!(maps.forward.is_empty());
}

#[test]
fn basic_sanitizer_replaces_invalid_characters() {
    assert_eq!(
        basic_sanitize_tool_name(&String::from("bad/name")),
        "bad_name"
    );
}

#[test]
fn tool_choice_rewrite_copies_only_custom_tools() {
    let mut params = RequestParams {
        tools: Some(vec![
            ToolRecord {
                tool_type: Some(String::from("custom")),
                name: Some(String::from("bad/name")),
                description: Some(String::from("keep")),
                additional_fields: BTreeMap::new(),
            },
            ToolRecord {
                tool_type: Some(String::from("web_search")),
                name: Some(String::from("host/name")),
                description: None,
                additional_fields: BTreeMap::new(),
            },
            ToolRecord {
                tool_type: None,
                name: None,
                description: None,
                additional_fields: BTreeMap::new(),
            },
        ]),
        tool_choice: Some(ToolChoiceRecord {
            choice_type: Some(String::from("tool")),
            name: Some(String::from("bad/name")),
            additional_fields: BTreeMap::new(),
        }),
    };
    let maps = sanitize_tool_names_in_request(&mut params);
    assert_eq!(maps.forward["bad/name"], "bad_name");
    assert_rewritten_request(&params);
}

fn assert_rewritten_request(params: &RequestParams) {
    assert_eq!(
        params.tools.as_ref().unwrap()[0].name.as_deref(),
        Some("bad_name")
    );
    assert_eq!(
        params.tools.as_ref().unwrap()[1].name.as_deref(),
        Some("host/name")
    );
    assert_eq!(
        params.tool_choice.as_ref().unwrap().name.as_deref(),
        Some("bad_name")
    );
}

#[test]
fn request_and_messages_preserve_unmodeled_fields() {
    let mut params = serde_json::from_str::<RequestParams>(
        r#"{"tools":[{"type":"custom","name":"bad/name","description":"keep","input_schema":{"type":"object"},"cache_control":{"type":"ephemeral"}}],"tool_choice":{"type":"tool","name":"bad/name","extra":true}}"#,
    )
    .unwrap();
    sanitize_tool_names_in_request(&mut params);
    let tool = &params.tools.as_ref().unwrap()[0];
    assert_eq!(
        serde_json::to_string(&tool.additional_fields["input_schema"]).unwrap(),
        r#"{"type":"object"}"#
    );
    assert_eq!(
        serde_json::to_string(&tool.additional_fields["cache_control"]).unwrap(),
        r#"{"type":"ephemeral"}"#
    );
    assert_eq!(
        serde_json::to_string(&params.tool_choice.as_ref().unwrap().additional_fields["extra"])
            .unwrap(),
        "true"
    );

    let parsed = serde_json::from_str::<Vec<MessageRecord>>(
        r#"[{"tool_calls":[{"id":"call-1","type":"function","function":{"name":"bad/name","arguments":"{}"}}],"content":[{"type":"text","text":"sibling"}]}]"#,
    )
    .unwrap();
    let rewritten = rewrite_tool_names_in_messages(
        &parsed,
        &BTreeMap::from([(String::from("bad/name"), String::from("bad_name"))]),
    );
    let tool_call = &rewritten[0].tool_calls.as_ref().unwrap()[0];
    assert_eq!(
        serde_json::to_string(&tool_call.additional_fields["type"]).unwrap(),
        r#""function""#
    );
    assert_eq!(
        serde_json::to_string(
            &rewritten[0].content.as_ref().unwrap()[0].additional_fields["text"],
        )
        .unwrap(),
        r#""sibling""#
    );
}

fn message_records() -> Vec<MessageRecord> {
    vec![
        MessageRecord {
            tool_calls: Some(vec![
                ToolCallRecord {
                    id: Some(String::from("1")),
                    function: Some(FunctionRecord {
                        name: Some(String::from("bad/name")),
                        arguments: Some(String::from("{}")),
                    }),
                    name: None,
                    additional_fields: BTreeMap::new(),
                },
                ToolCallRecord {
                    id: None,
                    name: Some(String::from("bad/name")),
                    function: None,
                    additional_fields: BTreeMap::new(),
                },
            ]),
            function_call: None,
            content: None,
        },
        MessageRecord {
            function_call: Some(FunctionRecord {
                name: Some(String::from("bad/name")),
                arguments: Some(String::from("{}")),
            }),
            tool_calls: None,
            content: None,
        },
    ]
}

#[test]
fn tool_message_rewrite_preserves_unrelated_entries() {
    let forward = BTreeMap::from([(String::from("bad/name"), String::from("bad_name"))]);
    let messages = message_records();
    let rewritten = rewrite_tool_names_in_messages(&messages, &forward);
    assert_eq!(
        rewritten[0].tool_calls.as_ref().unwrap()[0]
            .function
            .as_ref()
            .unwrap()
            .name
            .as_deref(),
        Some("bad_name")
    );
    assert_eq!(
        rewritten[0].tool_calls.as_ref().unwrap()[1].name.as_deref(),
        Some("bad/name")
    );
    assert_eq!(
        rewritten[1].function_call.as_ref().unwrap().name.as_deref(),
        Some("bad_name")
    );
}
