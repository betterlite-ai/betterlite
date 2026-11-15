use rstest::rstest;

use super::fixtures::{
    ScriptedContinuation, capture, chat_tool_call, dict, final_answer, hook, json, object_response,
    outcome, request, run_loop, skill_files, tool_block, tool_use,
};
use crate::api::skills::continuation::ContinuationLoop as _;
use crate::api::skills::continuation::content::assistant_message;
use crate::api::skills::continuation::messages::ContinuationRequest;
use crate::api::skills::hook::response::ContentBlock;

#[tokio::test]
async fn loop_messages_first_request_carries_the_model_messages_and_tools() {
    let hook = hook(10, vec![outcome("ran")]);
    let response = dict(
        r#"{"stop_reason":"tool_use","content":[
            {"type":"text","text":"thinking"},
            {"type":"tool_use","id":"t1","name":"litellm_code_execution","input":{"code":"print(1)"}}
        ]}"#,
    );
    let (result, continuation) =
        run_loop(&hook, response, vec![Ok(Some(final_answer("last")))]).await;
    result.unwrap();
    assert_eq!(
        continuation.requests(),
        vec![ContinuationRequest {
            model: "claude-sonnet".to_owned(),
            messages: vec![
                json(r#"{"role":"user","content":"make a chart"}"#),
                json(
                    r#"{"role":"assistant","content":[
                        {"type":"text","text":"thinking"},
                        {"type":"tool_use","id":"t1","name":"litellm_code_execution","input":{"code":"print(1)"}}
                    ]}"#
                ),
                json(
                    r#"{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ran"}]}"#
                ),
            ],
            tools: vec![json(r#"{"name":"litellm_code_execution"}"#)],
            max_tokens: 4096,
        }]
    );
}

#[rstest]
#[case::default(None, 4096)]
#[case::explicit(Some(1000), 1000)]
#[case::explicit_zero(Some(0), 0)]
#[tokio::test]
async fn loop_messages_max_tokens_defaults_to_4096_and_passes_an_explicit_value(
    #[case] given: Option<u64>,
    #[case] expected: u64,
) {
    let hook = hook(10, vec![outcome("ran")]);
    let continuation = ScriptedContinuation::new(vec![Ok(Some(final_answer("last")))]);
    let mut data = request();
    data.max_tokens = given;
    let files = skill_files(&[("a.py", "a")]);
    let response = tool_use("first", "t1", "litellm_code_execution", "{}");
    hook.execute_messages_loop(&data, response, &files, &continuation)
        .await
        .unwrap();
    assert_eq!(continuation.requests()[0].max_tokens, expected);
}

#[tokio::test]
async fn loop_messages_every_round_sends_a_snapshot_of_the_growing_conversation() {
    let hook = hook(10, vec![outcome("one"), outcome("two")]);
    let first = tool_use("first", "t1", "litellm_code_execution", r#"{"code":"a"}"#);
    let second = tool_use("second", "t2", "litellm_code_execution", r#"{"code":"b"}"#);
    let (result, continuation) = run_loop(
        &hook,
        first,
        vec![Ok(Some(second)), Ok(Some(final_answer("last")))],
    )
    .await;
    result.unwrap();
    let requests = continuation.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].messages.len(), 3);
    assert_eq!(requests[1].messages.len(), 5);
    assert_eq!(requests[1].messages[..3], requests[0].messages[..]);
    assert_eq!(
        requests[1].messages[3],
        json(
            r#"{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"litellm_code_execution","input":{"code":"b"}}]}"#
        )
    );
    assert_eq!(
        requests[1].messages[4],
        json(
            r#"{"role":"user","content":[{"type":"tool_result","tool_use_id":"t2","content":"two"}]}"#
        )
    );
}

#[tokio::test]
async fn loop_messages_only_model_messages_tools_and_max_tokens_are_forwarded() {
    let hook = hook(10, vec![outcome("ran")]);
    let response = tool_use("first", "t1", "litellm_code_execution", "{}");
    let (result, continuation) =
        run_loop(&hook, response, vec![Ok(Some(final_answer("last")))]).await;
    result.unwrap();
    let ContinuationRequest {
        model,
        messages,
        tools,
        max_tokens,
    } = continuation.requests().remove(0);
    assert_eq!(
        (model.as_str(), messages.len(), tools.len(), max_tokens),
        ("claude-sonnet", 3, 1, 4096)
    );
}

#[tokio::test]
async fn loop_messages_dict_entries_that_are_not_objects_become_text_blocks() {
    let hook = hook(10, vec![outcome("ran")]);
    let response = dict(
        r#"{"stop_reason":"tool_use","content":[
            "plain text",
            5,
            {"type":"tool_use","id":"t1","name":"litellm_code_execution","input":{}}
        ]}"#,
    );
    let (result, continuation) =
        run_loop(&hook, response, vec![Ok(Some(final_answer("last")))]).await;
    result.unwrap();
    assert_eq!(
        continuation.requests()[0].messages[1],
        json(
            r#"{"role":"assistant","content":[
                {"type":"text","text":"\"plain text\""},
                {"type":"text","text":"5"},
                {"type":"tool_use","id":"t1","name":"litellm_code_execution","input":{}}
            ]}"#
        )
    );
}

#[rstest]
#[case::absent(r#"{"stop_reason":"end_turn"}"#)]
#[case::null(r#"{"stop_reason":"end_turn","content":null}"#)]
#[case::not_a_list(r#"{"stop_reason":"end_turn","content":"just text"}"#)]
fn loop_messages_dict_without_a_content_list_gives_an_empty_assistant_content(#[case] body: &str) {
    assert_eq!(
        assistant_message(&dict(body)),
        json(r#"{"role":"assistant","content":[]}"#)
    );
}

#[rstest]
#[case::text_block(
    ContentBlock {
        kind: "text".to_owned(),
        id: None,
        name: None,
        input: json("null"),
        text: Some("hello".to_owned()),
    },
    r#"{"type":"text","input":null,"text":"hello"}"#
)]
#[case::tool_block(
    tool_block("t1", "litellm_code_execution", r#"{"code":"x"}"#),
    r#"{"type":"tool_use","id":"t1","name":"litellm_code_execution","input":{"code":"x"}}"#
)]
fn loop_messages_typed_blocks_keep_only_their_present_fields(
    #[case] block: ContentBlock,
    #[case] expected: &str,
) {
    let response = object_response(vec![block], "tool_use");
    assert_eq!(
        assistant_message(&response),
        json(&format!(r#"{{"role":"assistant","content":[{expected}]}}"#))
    );
}

#[tokio::test]
async fn loop_messages_choices_only_tool_calls_give_an_empty_assistant_content() {
    let hook = hook(10, vec![outcome("ran")]);
    let (result, continuation) = run_loop(
        &hook,
        chat_tool_call(r#"{"code":"print(1)"}"#),
        vec![Ok(Some(final_answer("last")))],
    )
    .await;
    result.unwrap();
    let requests = continuation.requests();
    assert_eq!(
        requests[0].messages[1],
        json(r#"{"role":"assistant","content":[]}"#)
    );
    assert_eq!(
        requests[0].messages[2],
        json(
            r#"{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":"ran"}]}"#
        )
    );
}

#[tokio::test]
async fn loop_messages_the_continuation_calls_are_numbered_from_two() {
    let (logs, _guard) = capture();
    let hook = hook(10, vec![outcome("one"), outcome("two")]);
    let first = tool_use("first", "t1", "litellm_code_execution", "{}");
    let second = tool_use("second", "t2", "litellm_code_execution", "{}");
    let (result, _continuation) = run_loop(
        &hook,
        first,
        vec![Ok(Some(second)), Ok(Some(final_answer("last")))],
    )
    .await;
    result.unwrap();
    let numbered: Vec<String> = logs
        .messages()
        .into_iter()
        .filter(|message| message.contains("Making LLM call"))
        .collect();
    assert_eq!(
        numbered,
        vec![
            "SkillsInjectionHook: Making LLM call iteration 2".to_owned(),
            "SkillsInjectionHook: Making LLM call iteration 3".to_owned(),
        ]
    );
}

#[tokio::test]
async fn loop_messages_the_final_response_is_the_last_continuation_reply() {
    let hook = hook(10, vec![outcome("ran")]);
    let response = tool_use("first", "t1", "litellm_code_execution", "{}");
    let (result, _continuation) =
        run_loop(&hook, response, vec![Ok(Some(final_answer("last")))]).await;
    assert_eq!(result.unwrap(), final_answer("last"));
}
