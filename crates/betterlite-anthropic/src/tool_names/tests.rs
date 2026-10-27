#![cfg(test)]

use std::collections::BTreeMap;

use super::{
    MessageHead, Record, RequestParams, basic_sanitize_tool_name, build_tool_name_maps,
    rewrite_tool_names_in_messages, sanitize_tool_names_in_request,
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
    let mut params = serde_json::from_str::<RequestParams>(
        r#"{"tools":[{"type":"custom","name":"bad/name","description":"keep","input_schema":{"type":"object"},"cache_control":{"type":"ephemeral"}},{"type":"web_search","name":"bad/name"},{"type":"custom","description":"nameless"}],"tool_choice":{"type":"tool","name":"bad/name","extra":true}}"#,
    )
    .unwrap();
    let maps = sanitize_tool_names_in_request(&mut params);
    assert_eq!(maps.forward["bad/name"], "bad_name");
    assert_eq!(
        serde_json::to_string(&params).unwrap(),
        r#"{"tools":[{"type":"custom","name":"bad_name","cache_control":{"type":"ephemeral"},"description":"keep","input_schema":{"type":"object"}},{"type":"web_search","name":"bad/name"},{"type":"custom","description":"nameless"}],"tool_choice":{"type":"tool","name":"bad_name","extra":true}}"#
    );
}

#[test]
fn tool_choice_without_rewrites_stays_unchanged() {
    let source = r#"{"tools":[{"type":"custom","name":"good"}],"tool_choice":{"type":"tool","name":"good"}}"#;
    let mut params = serde_json::from_str::<RequestParams>(source).unwrap();
    assert!(
        sanitize_tool_names_in_request(&mut params)
            .forward
            .is_empty()
    );
    assert_eq!(serde_json::to_string(&params).unwrap(), source);
}

#[test]
fn tool_message_rewrite_preserves_unrelated_entries() {
    let forward = BTreeMap::from([(String::from("bad/name"), String::from("bad_name"))]);
    let messages = serde_json::from_str::<Vec<Record<MessageHead>>>(
        r#"[{"role":"assistant","tool_calls":[{"id":"1","type":"function","function":{"name":"bad/name","arguments":"{}"}},{"name":"bad/name"}],"content":[{"type":"tool_use","name":"bad/name"}]},{"role":"assistant","function_call":{"name":"bad/name","arguments":"{}"}},{"function_call":{"name":7}}]"#,
    )
    .unwrap();
    let rewritten = rewrite_tool_names_in_messages(&messages, &forward);
    assert_eq!(
        serde_json::to_string(&rewritten).unwrap(),
        r#"[{"tool_calls":[{"function":{"name":"bad_name","arguments":"{}"},"id":"1","type":"function"},{"name":"bad/name"}],"content":[{"type":"tool_use","name":"bad/name"}],"role":"assistant"},{"function_call":{"name":"bad_name","arguments":"{}"},"role":"assistant"},{"function_call":{"name":7}}]"#
    );
}

#[test]
fn empty_forward_map_returns_messages_unchanged() {
    let source = r#"[{"function_call":{"name":"bad/name"}}]"#;
    let messages = serde_json::from_str::<Vec<Record<MessageHead>>>(source).unwrap();
    let rewritten = rewrite_tool_names_in_messages(&messages, &BTreeMap::new());
    assert_eq!(serde_json::to_string(&rewritten).unwrap(), source);
}
