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
