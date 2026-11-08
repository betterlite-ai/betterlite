use std::iter::repeat_n;

use crate::anthropic_invokes::invoke::{
    InvokeError, MAX_SERVER_RESULT_BYTES, MAX_SERVER_RESULTS, MAX_TOOL_CALLS,
    to_anthropic_tool_invokes,
};
use crate::arguments::json::{Json, Object};

use super::{list, object};

fn invoke(tools: &str) -> Result<Vec<Object>, InvokeError> {
    to_anthropic_tool_invokes(&list(tools), &[], &[])
}

fn invoke_single(tool: &str) -> Result<Vec<Object>, InvokeError> {
    invoke(&format!("[{tool}]"))
}

fn invoke_with(
    tools: &str,
    web_search: &str,
    tool_results: &str,
) -> Result<Vec<Object>, InvokeError> {
    to_anthropic_tool_invokes(&list(tools), &list(web_search), &list(tool_results))
}

fn repeated_calls(tool_id: &str, count: usize) -> Vec<Json> {
    let call = object(&format!(
        r#"{{"id":"{tool_id}","type":"function","function":{{"name":"x","arguments":"{{}}"}}}}"#
    ));
    repeat_n(Json::Object(call), count).collect()
}

fn server_results(tool_id: &str, data_len: usize, count: usize) -> Vec<Json> {
    let payload = "x".repeat(data_len);
    let result = object(&format!(
        r#"{{"tool_use_id":"{tool_id}","data":"{payload}"}}"#
    ));
    repeat_n(Json::Object(result), count).collect()
}

fn cached_tool(cache_control: Option<&str>) -> String {
    cache_control.map_or_else(
        || r#"{"id":"c1","type":"function","function":{"name":"x","arguments":"{}"}}"#.to_owned(),
        |value| {
            format!(
                r#"{{"id":"c1","type":"function","function":{{"name":"x","arguments":"{{}}"}},"cache_control":{value}}}"#
            )
        },
    )
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_function_call_becomes_a_tool_use() {
    let tools = invoke(
        r#"[{"id":"call_abc","type":"function","function":{"name":"weather","arguments":"{\"city\":\"Paris\"}"}}]"#,
    );
    assert_eq!(
        tools.unwrap(),
        vec![object(
            r#"{"type":"tool_use","id":"call_abc","name":"weather","input":{"city":"Paris"}}"#
        )]
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_an_invalid_id_is_sanitized() {
    let blocks = invoke_single(
        r#"{"id":"call.123/abc","type":"function","function":{"name":"weather","arguments":"{}"}}"#,
    );
    assert_eq!(
        blocks.unwrap(),
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
    assert!(invoke(tools).unwrap().is_empty());
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_non_object_tool_calls_are_skipped() {
    let blocks = invoke(
        r#"["nope",{"id":"c1","type":"function","function":{"name":"x","arguments":"{}"}}]"#,
    );
    assert_eq!(
        blocks.unwrap(),
        vec![object(
            r#"{"type":"tool_use","id":"c1","name":"x","input":{}}"#
        )]
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_skipped_tool_does_not_stop_later_calls() {
    let blocks = invoke(
        r#"[{"id":"c0","type":"web_search","function":{"name":"x","arguments":"{}"}},{"id":"c1","type":"function","function":{"name":"x","arguments":"{}"}}]"#,
    );
    assert_eq!(
        blocks.unwrap(),
        vec![object(
            r#"{"type":"tool_use","id":"c1","name":"x","input":{}}"#
        )]
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_plain_id_with_a_result_stays_a_tool_use() {
    let blocks = invoke_with(
        r#"[{"id":"call_1","type":"function","function":{"name":"search","arguments":"{}"}}]"#,
        r#"[{"tool_use_id":"call_1","source":"web"}]"#,
        "[]",
    );
    assert_eq!(
        blocks.unwrap(),
        vec![object(
            r#"{"type":"tool_use","id":"call_1","name":"search","input":{}}"#
        )]
    );
}

#[rstest::rstest]
#[case::empty_function(r#"{"id":"c1","type":"function","function":{}}"#)]
#[case::null_fields(r#"{"id":"c1","type":"function","function":{"name":null,"arguments":null}}"#)]
#[case::no_function(r#"{"id":"c1","type":"function"}"#)]
#[case::null_function(r#"{"id":"c1","type":"function","function":null}"#)]
fn anthropic_invokes_behavior_tests_behavior_a_missing_name_is_null_and_arguments_are_empty(
    #[case] tool: &str,
) {
    assert_eq!(
        invoke_single(tool).unwrap(),
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
    let tool = format!(
        r#"{{"id":"c1","type":"function","function":{{"name":"x","arguments":"{arguments}"}}}}"#
    );
    assert_eq!(
        invoke_single(&tool).unwrap(),
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
    let tool = format!(
        r#"{{"id":"c1","type":"function","function":{{"name":"x","arguments":"{}"}}}}"#,
        arguments.replace('"', "\\\"")
    );
    let blocks = invoke_single(&tool).unwrap();
    let input = blocks.first().and_then(|block| block.get("input")).unwrap();
    assert_eq!(input, &serde_json::from_str::<Json>(arguments).unwrap());
}

#[rstest::rstest]
#[case::function(r#"{"id":"c1","type":"function","function":5}"#, "function")]
#[case::name(
    r#"{"id":"c1","type":"function","function":{"name":5,"arguments":"{}"}}"#,
    "name"
)]
#[case::arguments(
    r#"{"id":"c1","type":"function","function":{"name":"x","arguments":5}}"#,
    "arguments"
)]
#[case::id(
    r#"{"type":"function","function":{"name":"x","arguments":"{}"}}"#,
    "id"
)]
#[case::null_id(
    r#"{"id":null,"type":"function","function":{"name":"x","arguments":"{}"}}"#,
    "id"
)]
#[case::number_id(
    r#"{"id":5,"type":"function","function":{"name":"x","arguments":"{}"}}"#,
    "id"
)]
fn anthropic_invokes_behavior_tests_behavior_a_wrong_typed_tool_field_is_named(
    #[case] tool: &str,
    #[case] expected: &'static str,
) {
    assert!(matches!(
        invoke_single(tool).unwrap_err(),
        InvokeError::Field { field } if field == expected
    ));
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_malformed_arguments_are_an_error() {
    let tool = r#"{"id":"c1","type":"function","function":{"name":"x","arguments":"not json"}}"#;
    assert!(matches!(
        invoke_single(tool).unwrap_err(),
        InvokeError::Arguments(_)
    ));
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_malformed_arguments_win_over_a_missing_id() {
    let tool = r#"{"type":"function","function":{"name":"x","arguments":"not json"}}"#;
    assert!(matches!(
        invoke_single(tool).unwrap_err(),
        InvokeError::Arguments(_)
    ));
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_cache_control_object_is_copied_onto_the_tool_use() {
    let tool = cached_tool(Some(r#"{"type":"ephemeral","ttl":"5m"}"#));
    assert_eq!(
        invoke_single(&tool).unwrap(),
        vec![object(
            r#"{"type":"tool_use","id":"c1","name":"x","input":{},"cache_control":{"type":"ephemeral","ttl":"5m"}}"#
        )]
    );
}

#[rstest::rstest]
#[case::text(Some(r#""ephemeral""#))]
#[case::list(Some(r#"[{"type":"ephemeral"}]"#))]
#[case::number(Some("5"))]
#[case::boolean(Some("true"))]
#[case::null(Some("null"))]
#[case::missing(None)]
fn anthropic_invokes_behavior_tests_behavior_a_non_object_cache_control_is_ignored(
    #[case] cache_control: Option<&str>,
) {
    assert_eq!(
        invoke_single(&cached_tool(cache_control)).unwrap(),
        vec![object(
            r#"{"type":"tool_use","id":"c1","name":"x","input":{}}"#
        )]
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_cache_control_extra_keys_are_preserved() {
    let tool = cached_tool(Some(r#"{"type":"ephemeral","ttl":"1h","zzz":1}"#));
    assert_eq!(
        invoke_single(&tool).unwrap(),
        vec![object(
            r#"{"type":"tool_use","id":"c1","name":"x","input":{},"cache_control":{"type":"ephemeral","ttl":"1h","zzz":1}}"#
        )]
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_server_tool_result_follows_its_use() {
    let tools = r#"[{"id":"call_1","type":"function","function":{"name":"a","arguments":"{}"}},{"id":"srvtoolu_abc","type":"function","function":{"name":"search","arguments":"{\"q\":\"x\"}"}},{"id":"call_3","type":"function","function":{"name":"b","arguments":"{}"}}]"#;
    let web = r#"[{"tool_use_id":"srvtoolu_abc","type":"web_search_tool_result","content":[{"type":"text","text":"r"}]}]"#;
    assert_eq!(
        invoke_with(tools, web, "[]").unwrap(),
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
    let tools = invoke_with(
        r#"[{"id":"srvtoolu_abc","type":"function","function":{"name":"code","arguments":"{}"}}]"#,
        "[]",
        r#"[{"tool_use_id":"srvtoolu_abc","type":"code_execution_tool_result"}]"#,
    );
    assert_eq!(
        tools.unwrap(),
        vec![
            object(r#"{"type":"server_tool_use","id":"srvtoolu_abc","name":"code","input":{}}"#),
            object(r#"{"tool_use_id":"srvtoolu_abc","type":"code_execution_tool_result"}"#),
        ]
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_the_web_search_result_wins_over_the_tool_result() {
    let blocks = invoke_with(
        r#"[{"id":"srvtoolu_abc","type":"function","function":{"name":"search","arguments":"{}"}}]"#,
        r#"[{"tool_use_id":"srvtoolu_abc","source":"web"}]"#,
        r#"[{"tool_use_id":"srvtoolu_abc","source":"tool"}]"#,
    );
    assert_eq!(
        blocks.unwrap().get(1),
        Some(&object(r#"{"tool_use_id":"srvtoolu_abc","source":"web"}"#))
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_server_tool_use_keeps_its_id_unsanitized() {
    let blocks = invoke_with(
        r#"[{"id":"srvtoolu_a.b","type":"function","function":{"name":"search","arguments":"{}"}}]"#,
        r#"[{"tool_use_id":"srvtoolu_a.b","source":"web"}]"#,
        "[]",
    );
    assert_eq!(
        blocks.unwrap(),
        vec![
            object(r#"{"type":"server_tool_use","id":"srvtoolu_a.b","name":"search","input":{}}"#),
            object(r#"{"tool_use_id":"srvtoolu_a.b","source":"web"}"#),
        ]
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_server_tool_without_a_result_becomes_a_tool_use() {
    let blocks = invoke_single(
        r#"{"id":"srvtoolu_a.b","type":"function","function":{"name":"search","arguments":"{}"}}"#,
    );
    assert_eq!(
        blocks.unwrap(),
        vec![object(
            r#"{"type":"tool_use","id":"srvtoolu_a_b","name":"search","input":{}}"#
        )]
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_server_tool_use_ignores_cache_control() {
    let blocks = invoke_with(
        r#"[{"id":"srvtoolu_abc","type":"function","function":{"name":"search","arguments":"{}"},"cache_control":{"type":"ephemeral"}}]"#,
        r#"[{"tool_use_id":"srvtoolu_abc","source":"web"}]"#,
        "[]",
    );
    assert_eq!(
        blocks.unwrap(),
        vec![
            object(r#"{"type":"server_tool_use","id":"srvtoolu_abc","name":"search","input":{}}"#),
            object(r#"{"tool_use_id":"srvtoolu_abc","source":"web"}"#),
        ]
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_the_server_tool_result_is_preserved_verbatim() {
    let blocks = invoke_with(
        r#"[{"id":"srvtoolu_abc","type":"function","function":{"name":"search","arguments":"{}"}}]"#,
        r#"[{"tool_use_id":"srvtoolu_abc","type":"web_search_tool_result","content":[{"type":"text","text":"r"}],"encrypted_content":"sig","extra":[1,2]}]"#,
        "[]",
    );
    assert_eq!(
        blocks.unwrap(),
        vec![
            object(r#"{"type":"server_tool_use","id":"srvtoolu_abc","name":"search","input":{}}"#),
            object(
                r#"{"tool_use_id":"srvtoolu_abc","type":"web_search_tool_result","content":[{"type":"text","text":"r"}],"encrypted_content":"sig","extra":[1,2]}"#
            ),
        ]
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_the_tool_call_limit_is_accepted() {
    let calls = repeated_calls("call_1", MAX_TOOL_CALLS);
    assert_eq!(
        to_anthropic_tool_invokes(&calls, &[], &[]).unwrap().len(),
        MAX_TOOL_CALLS
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_too_many_tool_calls_are_refused() {
    let calls = repeated_calls("call_1", MAX_TOOL_CALLS.saturating_add(1));
    assert!(matches!(
        to_anthropic_tool_invokes(&calls, &[], &[]).unwrap_err(),
        InvokeError::TooManyToolCalls { limit } if limit == MAX_TOOL_CALLS
    ));
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_the_server_result_count_limit_is_accepted() {
    let results = server_results("srvtoolu_x", 0, MAX_SERVER_RESULTS);
    assert!(
        to_anthropic_tool_invokes(&[], &results, &[])
            .unwrap()
            .is_empty()
    );
}

#[rstest::rstest]
#[case::web_search("web_search_results")]
#[case::tool_results("tool_results")]
fn anthropic_invokes_behavior_tests_behavior_too_many_server_results_are_refused(
    #[case] field: &'static str,
) {
    let results = server_results("srvtoolu_x", 0, MAX_SERVER_RESULTS.saturating_add(1));
    let error = if field == "web_search_results" {
        to_anthropic_tool_invokes(&[], &results, &[])
    } else {
        to_anthropic_tool_invokes(&[], &[], &results)
    }
    .unwrap_err();
    assert!(matches!(
        error,
        InvokeError::TooManyServerResults { field: refused, limit }
            if refused == field && limit == MAX_SERVER_RESULTS
    ));
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_repeated_server_calls_over_the_byte_cap_are_refused() {
    let calls = repeated_calls("srvtoolu_shared", MAX_TOOL_CALLS);
    let results = server_results("srvtoolu_shared", 0x4000, 1);
    assert!(matches!(
        to_anthropic_tool_invokes(&calls, &results, &[]).unwrap_err(),
        InvokeError::ServerResultsTooLarge { limit } if limit == MAX_SERVER_RESULT_BYTES
    ));
}

// 44 bytes of object head and keys plus 8148 text bytes is 8192 per copy, so 1024 copies land exactly on the cap
#[test]
fn anthropic_invokes_behavior_tests_behavior_server_results_exactly_at_the_byte_cap_are_copied() {
    let calls = repeated_calls("srvtoolu_shared", MAX_TOOL_CALLS);
    let results = server_results("srvtoolu_shared", 0x1FD4, 1);
    let blocks = to_anthropic_tool_invokes(&calls, &results, &[]).unwrap();
    assert_eq!(blocks.len(), MAX_TOOL_CALLS.saturating_mul(2));
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_repeated_server_calls_under_the_byte_cap_are_copied() {
    let calls = repeated_calls("srvtoolu_shared", MAX_TOOL_CALLS);
    let results = server_results("srvtoolu_shared", 0x1000, 1);
    let blocks = to_anthropic_tool_invokes(&calls, &results, &[]).unwrap();
    assert_eq!(blocks.len(), MAX_TOOL_CALLS.saturating_mul(2));
}
