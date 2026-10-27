#![cfg(test)]

use std::collections::BTreeMap;
use std::thread;

use super::wire::{Message, Name, RequestParams};
use super::{
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

#[rstest]
#[case::invalid(r#""bad/name""#, r#""bad_name""#)]
#[case::empty(r#""""#, r#""""#)]
#[case::number("42", "42")]
#[case::null("null", "null")]
#[case::object(r#"{"name":"bad/name"}"#, r#"{"name":"bad/name"}"#)]
fn basic_sanitizer_preserves_nonstrings(#[case] source: &str, #[case] expected: &str) {
    let name = serde_json::from_str::<Name>(source).unwrap();
    assert_eq!(
        serde_json::to_string(&basic_sanitize_tool_name(&name)).unwrap(),
        expected
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
fn hosted_tools_do_not_reserve_names() {
    let mut params = serde_json::from_str::<RequestParams>(
        r#"{"tools":[{"type":"web_search","name":"foo_bar"},{"type":"custom","name":"foo/bar"}]}"#,
    )
    .unwrap();
    let maps = sanitize_tool_names_in_request(&mut params);
    assert_eq!(maps.forward["foo/bar"], "foo_bar");
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
    let messages = serde_json::from_str::<Vec<Message>>(
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
    let messages = serde_json::from_str::<Vec<Message>>(source).unwrap();
    let rewritten = rewrite_tool_names_in_messages(&messages, &BTreeMap::new());
    assert_eq!(serde_json::to_string(&rewritten).unwrap(), source);
}

#[test]
fn malformed_entries_and_nulls_are_preserved() {
    let source = r#"{"tools":[null,3,[],{"type":"custom","name":"bad/name"},{"type":"custom","name":null}],"tool_choice":false}"#;
    let mut params: RequestParams = serde_json::from_str(source).unwrap();
    let maps = sanitize_tool_names_in_request(&mut params);
    assert_eq!(maps.forward["bad/name"], "bad_name");
    assert_eq!(
        serde_json::to_string(&params).unwrap(),
        source.replace("bad/name", "bad_name")
    );
}

#[rstest]
#[case::missing("{}")]
#[case::empty(r#"{"tools":[]}"#)]
#[case::null(r#"{"tools":null}"#)]
#[case::wrong_shape(r#"{"tools":{"type":"custom","name":"bad/name"}}"#)]
#[case::hosted(r#"{"tools":[{"type":"web_search","name":"bad/name"}]}"#)]
#[case::wrong_names(r#"{"tools":[{"type":"custom","name":42},{"type":"custom","name":{}},{"type":[],"name":"bad/name"}]}"#)]
fn ineligible_requests_are_unchanged(#[case] source: &str) {
    let mut params = serde_json::from_str::<RequestParams>(source).unwrap();
    assert_eq!(
        sanitize_tool_names_in_request(&mut params),
        super::ToolNameMaps::default()
    );
    assert_eq!(serde_json::to_string(&params).unwrap(), source);
}

#[rstest]
#[case::auto(r#"{"type":"auto","name":"bad/name"}"#)]
#[case::unmapped(r#"{"type":"tool","name":"valid"}"#)]
#[case::missing_name(r#"{"type":"tool"}"#)]
#[case::null_name(r#"{"type":"tool","name":null}"#)]
#[case::wrong_name(r#"{"type":"tool","name":[]}"#)]
#[case::raw(r#""auto""#)]
fn unrelated_choices_are_unchanged(#[case] choice: &str) {
    let source = format!(
        r#"{{"tools":[{{"type":"custom","name":"bad/name"}}],"tool_choice":{choice},"extra":{{"keep":true}}}}"#
    );
    let mut params = serde_json::from_str::<RequestParams>(&source).unwrap();
    sanitize_tool_names_in_request(&mut params);
    let expected = source.replacen("bad/name", "bad_name", 1);
    assert_eq!(serde_json::to_string(&params).unwrap(), expected);
}

#[test]
fn malformed_message_fields_do_not_hide_valid_calls() {
    let source = r#"[null,42,[],{"tool_calls":false,"function_call":{"name":"bad/name"}},{"tool_calls":[null,42,[],{"function":null},{"function":[]},{"function":{"name":null}},{"function":{"name":{}}},{"function":{"name":"bad/name","arguments":{"x":[1,null]}}}],"function_call":false}]"#;
    let messages = serde_json::from_str::<Vec<Message>>(source).unwrap();
    let forward = BTreeMap::from([("bad/name".to_owned(), "bad_name".to_owned())]);
    let rewritten = rewrite_tool_names_in_messages(&messages, &forward);
    assert_eq!(
        serde_json::to_string(&rewritten).unwrap(),
        source.replace("bad/name", "bad_name")
    );
    assert_eq!(serde_json::to_string(&messages).unwrap(), source);
}

#[test]
fn long_collisions_keep_suffixes_inside_limit() {
    let valid = "a".repeat(128);
    let invalid = format!("{valid}/");
    let second = format!("{valid}?");
    let names = vec![
        invalid.clone(),
        second.clone(),
        invalid.clone(),
        valid.clone(),
    ];
    let maps = build_tool_name_maps(&names);
    assert_eq!(maps.forward.len(), 2);
    assert_eq!(maps.forward[&invalid], format!("{}_2", "a".repeat(126)));
    assert_eq!(maps.forward[&second], format!("{}_3", "a".repeat(126)));
    assert!(!maps.reverse.contains_key(&valid));
}

#[test]
fn collisions_follow_input_order_and_skip_reserved_suffixes() {
    let names = ["a?b", "a/b", "a_b", "a_b_2", "a?b"].map(str::to_owned);
    let maps = build_tool_name_maps(&names);
    assert_eq!(maps.forward["a?b"], "a_b_3");
    assert_eq!(maps.forward["a/b"], "a_b_4");
    assert_eq!(maps.reverse.len(), 2);
}

#[test]
fn concurrent_requests_do_not_share_reverse_maps() {
    let source = r#"{"tools":[{"type":"custom","name":"foo/bar"}]}"#;
    let template = serde_json::from_str::<RequestParams>(source).unwrap();
    let copied = template.clone();
    thread::scope(|scope| {
        let changed = scope.spawn(move || {
            let mut params = copied;
            let maps = sanitize_tool_names_in_request(&mut params);
            assert!(
                !serde_json::to_string(&params)
                    .unwrap()
                    .contains(super::REVERSE_MAP_KEY)
            );
            maps
        });
        let unchanged = scope.spawn(|| {
            let mut params = serde_json::from_str::<RequestParams>(
                r#"{"tools":[{"type":"custom","name":"foo_bar"}]}"#,
            )
            .unwrap();
            sanitize_tool_names_in_request(&mut params)
        });
        assert_eq!(changed.join().unwrap().reverse["foo_bar"], "foo/bar");
        assert!(unchanged.join().unwrap().reverse.is_empty());
    });
    assert_eq!(serde_json::to_string(&template).unwrap(), source);
}
