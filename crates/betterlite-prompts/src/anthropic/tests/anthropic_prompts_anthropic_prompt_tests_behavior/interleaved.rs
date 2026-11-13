use rstest::rstest;

use crate::anthropic::assistant::fields::has_server_tool_call;
use crate::arguments::json::Json;

use super::fixtures::{converted, expected};

const THINKING_ONE: &str = r#"{"type":"thinking","thinking":"t1","signature":"s1"}"#;
const THINKING_TWO: &str = r#"{"type":"thinking","thinking":"t2","signature":"s2"}"#;
const THINKING_THREE: &str = r#"{"type":"thinking","thinking":"t3","signature":"s3"}"#;
const SEARCH_CALL: &str =
    r#"{"id":"srvtoolu_1","type":"function","function":{"name":"web_search","arguments":"{}"}}"#;
const SECOND_SEARCH_CALL: &str =
    r#"{"id":"srvtoolu_2","type":"function","function":{"name":"web_search","arguments":"{}"}}"#;
const CALC_CALL: &str =
    r#"{"id":"call_2","type":"function","function":{"name":"calc","arguments":"{}"}}"#;
const FIRST_RESULT: &str =
    r#"{"type":"web_search_tool_result","tool_use_id":"srvtoolu_1","content":[]}"#;
const SECOND_RESULT: &str =
    r#"{"type":"web_search_tool_result","tool_use_id":"srvtoolu_2","content":[]}"#;

fn request(thinking: &str, calls: &str, results: &str, content: &str) -> String {
    format!(
        r#"[{{"role":"user","content":"search"}},{{"role":"assistant","content":{content},"thinking_blocks":[{thinking}],"tool_calls":[{calls}],"provider_specific_fields":{{{results}}}}}]"#
    )
}

fn server_tool_use(tool_id: &str, name: &str) -> String {
    format!(r#"{{"type":"server_tool_use","id":"{tool_id}","name":"{name}","input":{{}}}}"#)
}

fn assistant_output(blocks: &[String]) -> Vec<Json> {
    let content = blocks.join(",");
    expected(&format!(
        r#"[{{"role":"user","content":[{{"type":"text","text":"search"}}]}},{{"role":"assistant","content":[{content}]}}]"#
    ))
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_interleaves_thinking_and_server_groups()
{
    let thinking = format!("{THINKING_ONE},{THINKING_TWO}");
    let calls = format!("{SEARCH_CALL},{CALC_CALL},{SECOND_SEARCH_CALL}");
    let results =
        format!(r#""web_search_results":[{FIRST_RESULT}],"tool_results":[{SECOND_RESULT}]"#);
    let input = request(&thinking, &calls, &results, r#""answer""#);
    let output = assistant_output(&[
        THINKING_ONE.to_owned(),
        server_tool_use("srvtoolu_1", "web_search"),
        FIRST_RESULT.to_owned(),
        THINKING_TWO.to_owned(),
        server_tool_use("srvtoolu_2", "web_search"),
        SECOND_RESULT.to_owned(),
        r#"{"type":"text","text":"answer"}"#.to_owned(),
        r#"{"type":"tool_use","id":"call_2","name":"calc","input":{}}"#.to_owned(),
    ]);
    assert_eq!(converted(&input, false).await, output);
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_more_thinking_than_groups() {
    let thinking = format!("{THINKING_ONE},{THINKING_TWO},{THINKING_THREE}");
    let results = format!(r#""web_search_results":[{FIRST_RESULT}]"#);
    let input = request(&thinking, SEARCH_CALL, &results, "null");
    let output = assistant_output(&[
        THINKING_ONE.to_owned(),
        server_tool_use("srvtoolu_1", "web_search"),
        FIRST_RESULT.to_owned(),
        THINKING_TWO.to_owned(),
        THINKING_THREE.to_owned(),
    ]);
    assert_eq!(converted(&input, false).await, output);
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_more_groups_than_thinking() {
    let calls = format!("{SEARCH_CALL},{SECOND_SEARCH_CALL}");
    let results =
        format!(r#""web_search_results":[{FIRST_RESULT}],"tool_results":[{SECOND_RESULT}]"#);
    let input = request(THINKING_ONE, &calls, &results, "null");
    let output = assistant_output(&[
        THINKING_ONE.to_owned(),
        server_tool_use("srvtoolu_1", "web_search"),
        FIRST_RESULT.to_owned(),
        server_tool_use("srvtoolu_2", "web_search"),
        SECOND_RESULT.to_owned(),
    ]);
    assert_eq!(converted(&input, false).await, output);
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_list_content_uses_sequential_mode() {
    let results = format!(r#""web_search_results":[{FIRST_RESULT}]"#);
    let input = request(
        THINKING_ONE,
        SEARCH_CALL,
        &results,
        r#"[{"type":"text","text":"answer"}]"#,
    );
    let output = assistant_output(&[
        THINKING_ONE.to_owned(),
        r#"{"type":"text","text":"answer"}"#.to_owned(),
        server_tool_use("srvtoolu_1", "web_search"),
        FIRST_RESULT.to_owned(),
    ]);
    assert_eq!(converted(&input, false).await, output);
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_server_calls_without_thinking_use_sequential_mode()
 {
    let input = format!(
        r#"[{{"role":"user","content":"search"}},{{"role":"assistant","content":null,"tool_calls":[{SEARCH_CALL}],"provider_specific_fields":{{"web_search_results":[{FIRST_RESULT}]}}}}]"#
    );
    let output = assistant_output(&[
        server_tool_use("srvtoolu_1", "web_search"),
        FIRST_RESULT.to_owned(),
    ]);
    assert_eq!(converted(&input, false).await, output);
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_server_calls_with_text_keep_the_text_first()
 {
    let results = format!(r#""web_search_results":[{FIRST_RESULT}]"#);
    let input = format!(
        r#"[{{"role":"user","content":"search"}},{{"role":"assistant","content":"answer","tool_calls":[{SEARCH_CALL}],"provider_specific_fields":{{{results}}}}}]"#
    );
    let output = assistant_output(&[
        r#"{"type":"text","text":"answer"}"#.to_owned(),
        server_tool_use("srvtoolu_1", "web_search"),
        FIRST_RESULT.to_owned(),
    ]);
    assert_eq!(converted(&input, false).await, output);
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_repeated_ids_are_dropped_across_merged_assistants()
 {
    let calls = format!("{SEARCH_CALL},{CALC_CALL}");
    let results = format!(r#""web_search_results":[{FIRST_RESULT}]"#);
    let first = format!(
        r#"{{"role":"assistant","content":null,"thinking_blocks":[{THINKING_ONE}],"tool_calls":[{calls}],"provider_specific_fields":{{{results}}}}}"#
    );
    let second = format!(
        r#"{{"role":"assistant","content":null,"thinking_blocks":[{THINKING_TWO}],"tool_calls":[{calls}],"provider_specific_fields":{{{results}}}}}"#
    );
    let input = format!(r#"[{{"role":"user","content":"search"}},{first},{second}]"#);
    let output = assistant_output(&[
        THINKING_ONE.to_owned(),
        server_tool_use("srvtoolu_1", "web_search"),
        FIRST_RESULT.to_owned(),
        r#"{"type":"tool_use","id":"call_2","name":"calc","input":{}}"#.to_owned(),
        THINKING_TWO.to_owned(),
        FIRST_RESULT.to_owned(),
    ]);
    assert_eq!(converted(&input, false).await, output);
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_interleaved_text_keeps_the_message_cache_control()
 {
    let results = format!(r#""web_search_results":[{FIRST_RESULT}]"#);
    let input = format!(
        r#"[{{"role":"user","content":"search"}},{{"role":"assistant","content":"answer","cache_control":{{"type":"ephemeral","ttl":"1h"}},"thinking_blocks":[{THINKING_ONE}],"tool_calls":[{SEARCH_CALL}],"provider_specific_fields":{{{results}}}}}]"#
    );
    let output = assistant_output(&[
        THINKING_ONE.to_owned(),
        server_tool_use("srvtoolu_1", "web_search"),
        FIRST_RESULT.to_owned(),
        r#"{"type":"text","text":"answer","cache_control":{"type":"ephemeral","ttl":"1h"}}"#
            .to_owned(),
    ]);
    assert_eq!(converted(&input, false).await, output);
}

#[rstest]
#[case::empty_thinking_list("[]")]
#[case::only_unsigned_thinking_blocks(r#"[{"type":"thinking","thinking":"x","signature":""}]"#)]
#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_filtered_thinking_still_interleaves(
    #[case] thinking_blocks: &str,
) {
    let results = format!(r#""web_search_results":[{FIRST_RESULT}]"#);
    let input = format!(
        r#"[{{"role":"user","content":"search"}},{{"role":"assistant","content":"answer","thinking_blocks":{thinking_blocks},"tool_calls":[{SEARCH_CALL}],"provider_specific_fields":{{{results}}}}}]"#
    );
    let output = assistant_output(&[
        server_tool_use("srvtoolu_1", "web_search"),
        FIRST_RESULT.to_owned(),
        r#"{"type":"text","text":"answer"}"#.to_owned(),
    ]);
    assert_eq!(converted(&input, false).await, output);
}

#[rstest]
#[case::no_tool_calls(None, false)]
#[case::a_server_tool_call(
    Some(r#"[{"id":"srvtoolu_1","type":"function","function":{"name":"web_search","arguments":"{}"}}]"#),
    true
)]
#[case::a_regular_tool_call(
    Some(r#"[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}]"#),
    false
)]
#[case::a_non_string_id(Some(r#"[{"id":5}]"#), false)]
#[case::a_non_object_call(Some("[7]"), false)]
#[case::an_empty_list(Some("[]"), false)]
fn anthropic_prompts_anthropic_prompt_tests_behavior_recognizes_server_tool_calls(
    #[case] calls: Option<&str>,
    #[case] expected: bool,
) {
    let parsed: Option<Vec<Json>> = calls.map(|raw| serde_json::from_str(raw).unwrap());
    assert_eq!(has_server_tool_call(parsed.as_deref()), expected);
}
