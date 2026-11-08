use crate::anthropic_invokes::invoke::{InvokeError, to_anthropic_tool_invokes};
use crate::arguments::json::Json;

use super::{list, object};

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_function_call_becomes_a_tool_use() {
    let calls = list(
        r#"[{"id":"call_abc","type":"function","function":{"name":"weather","arguments":"{\"city\":\"Paris\"}"}}]"#,
    );
    assert_eq!(
        to_anthropic_tool_invokes(&calls, &[], &[]).unwrap(),
        vec![object(
            r#"{"type":"tool_use","id":"call_abc","name":"weather","input":{"city":"Paris"}}"#
        )]
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_an_invalid_id_is_sanitized() {
    let calls = list(
        r#"[{"id":"call.123/abc","type":"function","function":{"name":"weather","arguments":"{}"}}]"#,
    );
    assert_eq!(
        to_anthropic_tool_invokes(&calls, &[], &[]).unwrap(),
        vec![object(
            r#"{"type":"tool_use","id":"call_123_abc","name":"weather","input":{}}"#
        )]
    );
}

#[rstest::rstest]
#[case::web_search(r#"[{"id":"c1","type":"web_search","function":{"name":"x","arguments":"{}"}}]"#)]
#[case::missing_type(r#"[{"id":"c1","function":{"name":"x","arguments":"{}"}}]"#)]
#[case::non_string_type(r#"[{"id":"c1","type":5,"function":{"name":"x","arguments":"{}"}}]"#)]
fn anthropic_invokes_behavior_tests_behavior_only_function_tools_are_converted(
    #[case] tools: &str,
) {
    assert!(
        to_anthropic_tool_invokes(&list(tools), &[], &[])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_non_object_tool_calls_are_skipped() {
    let calls =
        list(r#"["nope",{"id":"c1","type":"function","function":{"name":"x","arguments":"{}"}}]"#);
    assert_eq!(
        to_anthropic_tool_invokes(&calls, &[], &[]).unwrap(),
        vec![object(
            r#"{"type":"tool_use","id":"c1","name":"x","input":{}}"#
        )]
    );
}

#[rstest::rstest]
#[case::missing(r#"{"id":"c1","type":"function","function":{}}"#)]
#[case::null(r#"{"id":"c1","type":"function","function":{"name":null,"arguments":null}}"#)]
fn anthropic_invokes_behavior_tests_behavior_a_missing_name_is_null_and_arguments_are_empty(
    #[case] tool: &str,
) {
    let calls = list(&format!("[{tool}]"));
    assert_eq!(
        to_anthropic_tool_invokes(&calls, &[], &[]).unwrap(),
        vec![object(
            r#"{"type":"tool_use","id":"c1","name":null,"input":{}}"#
        )]
    );
}

#[rstest::rstest]
#[case::empty("")]
#[case::blank("  ")]
fn anthropic_invokes_behavior_tests_behavior_blank_arguments_become_an_empty_object(
    #[case] arguments: &str,
) {
    let calls = list(&format!(
        r#"[{{"id":"c1","type":"function","function":{{"name":"x","arguments":"{arguments}"}}}}]"#
    ));
    assert_eq!(
        to_anthropic_tool_invokes(&calls, &[], &[]).unwrap(),
        vec![object(
            r#"{"type":"tool_use","id":"c1","name":"x","input":{}}"#
        )]
    );
}

#[rstest::rstest]
#[case::list("[1,2]")]
#[case::text(r#""hi""#)]
fn anthropic_invokes_behavior_tests_behavior_arguments_keep_their_json_shape(
    #[case] arguments: &str,
) {
    let calls = list(&format!(
        r#"[{{"id":"c1","type":"function","function":{{"name":"x","arguments":"{}"}}}}]"#,
        arguments.replace('"', "\\\"")
    ));
    let blocks = to_anthropic_tool_invokes(&calls, &[], &[]).unwrap();
    let input = blocks.first().and_then(|block| block.get("input")).unwrap();
    assert_eq!(input, &serde_json::from_str::<Json>(arguments).unwrap());
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_non_object_function_is_a_field_error() {
    let calls = list(r#"[{"id":"c1","type":"function","function":5}]"#);
    assert!(matches!(
        to_anthropic_tool_invokes(&calls, &[], &[]).unwrap_err(),
        InvokeError::Field { field: "function" }
    ));
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_non_string_name_is_a_field_error() {
    let calls = list(r#"[{"id":"c1","type":"function","function":{"name":5,"arguments":"{}"}}]"#);
    assert!(matches!(
        to_anthropic_tool_invokes(&calls, &[], &[]).unwrap_err(),
        InvokeError::Field { field: "name" }
    ));
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_non_string_arguments_is_a_field_error() {
    let calls = list(r#"[{"id":"c1","type":"function","function":{"name":"x","arguments":5}}]"#);
    assert!(matches!(
        to_anthropic_tool_invokes(&calls, &[], &[]).unwrap_err(),
        InvokeError::Field { field: "arguments" }
    ));
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_malformed_arguments_are_an_error() {
    let calls =
        list(r#"[{"id":"c1","type":"function","function":{"name":"x","arguments":"not json"}}]"#);
    assert!(matches!(
        to_anthropic_tool_invokes(&calls, &[], &[]).unwrap_err(),
        InvokeError::Arguments(_)
    ));
}

#[rstest::rstest]
#[case::missing(r#"{"type":"function","function":{"name":"x","arguments":"{}"}}"#)]
#[case::null(r#"{"id":null,"type":"function","function":{"name":"x","arguments":"{}"}}"#)]
#[case::number(r#"{"id":5,"type":"function","function":{"name":"x","arguments":"{}"}}"#)]
fn anthropic_invokes_behavior_tests_behavior_a_missing_id_is_a_field_error(#[case] tool: &str) {
    let calls = list(&format!("[{tool}]"));
    assert!(matches!(
        to_anthropic_tool_invokes(&calls, &[], &[]).unwrap_err(),
        InvokeError::Field { field: "id" }
    ));
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_malformed_arguments_win_over_a_missing_id() {
    let calls = list(r#"[{"type":"function","function":{"name":"x","arguments":"not json"}}]"#);
    assert!(matches!(
        to_anthropic_tool_invokes(&calls, &[], &[]).unwrap_err(),
        InvokeError::Arguments(_)
    ));
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_cache_control_object_is_copied_onto_the_tool_use() {
    let calls = list(
        r#"[{"id":"c1","type":"function","function":{"name":"x","arguments":"{}"},"cache_control":{"type":"ephemeral","ttl":"5m"}}]"#,
    );
    assert_eq!(
        to_anthropic_tool_invokes(&calls, &[], &[]).unwrap(),
        vec![object(
            r#"{"type":"tool_use","id":"c1","name":"x","input":{},"cache_control":{"type":"ephemeral","ttl":"5m"}}"#
        )]
    );
}

#[rstest::rstest]
#[case::text(r#""ephemeral""#)]
#[case::list(r#"[{"type":"ephemeral"}]"#)]
#[case::null("null")]
#[case::missing("")]
fn anthropic_invokes_behavior_tests_behavior_a_non_object_cache_control_is_ignored(
    #[case] cache_control: &str,
) {
    let suffix = if cache_control.is_empty() {
        String::new()
    } else {
        format!(r#","cache_control":{cache_control}"#)
    };
    let calls = list(&format!(
        r#"[{{"id":"c1","type":"function","function":{{"name":"x","arguments":"{{}}"}}{suffix}}}]"#
    ));
    assert_eq!(
        to_anthropic_tool_invokes(&calls, &[], &[]).unwrap(),
        vec![object(
            r#"{"type":"tool_use","id":"c1","name":"x","input":{}}"#
        )]
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_cache_control_extra_keys_are_preserved() {
    let calls = list(
        r#"[{"id":"c1","type":"function","function":{"name":"x","arguments":"{}"},"cache_control":{"type":"ephemeral","ttl":"1h","zzz":1}}]"#,
    );
    assert_eq!(
        to_anthropic_tool_invokes(&calls, &[], &[]).unwrap(),
        vec![object(
            r#"{"type":"tool_use","id":"c1","name":"x","input":{},"cache_control":{"type":"ephemeral","ttl":"1h","zzz":1}}"#
        )]
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_server_tool_result_follows_its_use() {
    let calls = list(
        r#"[{"id":"call_1","type":"function","function":{"name":"a","arguments":"{}"}},{"id":"srvtoolu_abc","type":"function","function":{"name":"search","arguments":"{\"q\":\"x\"}"}},{"id":"call_3","type":"function","function":{"name":"b","arguments":"{}"}}]"#,
    );
    let web = list(
        r#"[{"tool_use_id":"srvtoolu_abc","type":"web_search_tool_result","content":[{"type":"text","text":"r"}]}]"#,
    );
    assert_eq!(
        to_anthropic_tool_invokes(&calls, &web, &[]).unwrap(),
        vec![
            object(r#"{"type":"tool_use","id":"call_1","name":"a","input":{}}"#),
            object(
                r#"{"type":"server_tool_use","id":"srvtoolu_abc","name":"search","input":{"q":"x"}}"#
            ),
            object(
                r#"{"tool_use_id":"srvtoolu_abc","type":"web_search_tool_result","content":[{"type":"text","text":"r"}]}"#
            ),
            object(r#"{"type":"tool_use","id":"call_3","name":"b","input":{}}"#),
        ]
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_server_tool_result_can_come_from_tool_results() {
    let calls = list(
        r#"[{"id":"srvtoolu_abc","type":"function","function":{"name":"code","arguments":"{}"}}]"#,
    );
    let tools = list(r#"[{"tool_use_id":"srvtoolu_abc","type":"code_execution_tool_result"}]"#);
    assert_eq!(
        to_anthropic_tool_invokes(&calls, &[], &tools).unwrap(),
        vec![
            object(r#"{"type":"server_tool_use","id":"srvtoolu_abc","name":"code","input":{}}"#),
            object(r#"{"tool_use_id":"srvtoolu_abc","type":"code_execution_tool_result"}"#),
        ]
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_the_web_search_result_wins_over_the_tool_result() {
    let calls = list(
        r#"[{"id":"srvtoolu_abc","type":"function","function":{"name":"search","arguments":"{}"}}]"#,
    );
    let web = list(r#"[{"tool_use_id":"srvtoolu_abc","source":"web"}]"#);
    let tools = list(r#"[{"tool_use_id":"srvtoolu_abc","source":"tool"}]"#);
    let blocks = to_anthropic_tool_invokes(&calls, &web, &tools).unwrap();
    assert_eq!(
        blocks.get(1),
        Some(&object(r#"{"tool_use_id":"srvtoolu_abc","source":"web"}"#))
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_server_tool_use_keeps_its_id_unsanitized() {
    let calls = list(
        r#"[{"id":"srvtoolu_a.b","type":"function","function":{"name":"search","arguments":"{}"}}]"#,
    );
    let web = list(r#"[{"tool_use_id":"srvtoolu_a.b","source":"web"}]"#);
    assert_eq!(
        to_anthropic_tool_invokes(&calls, &web, &[]).unwrap(),
        vec![
            object(r#"{"type":"server_tool_use","id":"srvtoolu_a.b","name":"search","input":{}}"#),
            object(r#"{"tool_use_id":"srvtoolu_a.b","source":"web"}"#),
        ]
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_server_tool_without_a_result_becomes_a_tool_use() {
    let calls = list(
        r#"[{"id":"srvtoolu_a.b","type":"function","function":{"name":"search","arguments":"{}"}}]"#,
    );
    assert_eq!(
        to_anthropic_tool_invokes(&calls, &[], &[]).unwrap(),
        vec![object(
            r#"{"type":"tool_use","id":"srvtoolu_a_b","name":"search","input":{}}"#
        )]
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_server_tool_use_ignores_cache_control() {
    let calls = list(
        r#"[{"id":"srvtoolu_abc","type":"function","function":{"name":"search","arguments":"{}"},"cache_control":{"type":"ephemeral"}}]"#,
    );
    let web = list(r#"[{"tool_use_id":"srvtoolu_abc","source":"web"}]"#);
    assert_eq!(
        to_anthropic_tool_invokes(&calls, &web, &[]).unwrap(),
        vec![
            object(r#"{"type":"server_tool_use","id":"srvtoolu_abc","name":"search","input":{}}"#),
            object(r#"{"tool_use_id":"srvtoolu_abc","source":"web"}"#),
        ]
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_the_server_tool_result_is_preserved_verbatim() {
    let calls = list(
        r#"[{"id":"srvtoolu_abc","type":"function","function":{"name":"search","arguments":"{}"}}]"#,
    );
    let web = list(
        r#"[{"tool_use_id":"srvtoolu_abc","type":"web_search_tool_result","content":[{"type":"text","text":"r"}],"encrypted_content":"sig","extra":[1,2]}]"#,
    );
    assert_eq!(
        to_anthropic_tool_invokes(&calls, &web, &[]).unwrap(),
        vec![
            object(r#"{"type":"server_tool_use","id":"srvtoolu_abc","name":"search","input":{}}"#),
            object(
                r#"{"tool_use_id":"srvtoolu_abc","type":"web_search_tool_result","content":[{"type":"text","text":"r"}],"encrypted_content":"sig","extra":[1,2]}"#
            ),
        ]
    );
}
