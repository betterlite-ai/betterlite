use rstest::rstest;
use tracing::Level;

use super::fixtures::{
    ScriptedContinuation, capture, chat_tool_call, dict, final_answer, hook, marker,
    object_response, outcome, request, tool_block, tool_use,
};
use crate::api::skills::continuation::ContinuationLoop;
use crate::api::skills::continuation::error::SkillsLoopError;

const DETECTED: &str = "SkillsInjectionHook: Detected tool call, starting execution loop";

#[tokio::test]
async fn detection_tools_response_without_tool_calls_returns_none() {
    let (logs, _guard) = capture();
    let hook = hook(10, Vec::new());
    let continuation = ScriptedContinuation::new(Vec::new());
    let result = hook
        .post_call_success(&request(), final_answer("first"), &continuation)
        .await
        .unwrap();
    assert_eq!(result, None);
    assert!(logs.entries().is_empty());
}

#[rstest]
#[case::other_name("other_tool")]
#[case::code_tool_with_suffix("litellm_code_execution_x")]
#[case::code_tool_with_prefix("x_litellm_code_execution")]
#[case::skill_prefix_without_underscore("litellm_skill")]
#[case::skill_prefix_in_the_middle("my_litellm_skill_abc")]
#[case::empty_name("")]
#[tokio::test]
async fn detection_tools_other_names_return_none_without_any_call(#[case] name: &str) {
    let (logs, _guard) = capture();
    let hook = hook(10, vec![outcome("ran")]);
    let continuation = ScriptedContinuation::new(Vec::new());
    let response = tool_use("first", "t1", name, "{}");
    let result = hook
        .post_call_success(&request(), response, &continuation)
        .await
        .unwrap();
    assert_eq!(result, None);
    assert!(logs.entries().is_empty());
    assert!(hook.executor.codes().is_empty());
    assert!(continuation.requests().is_empty());
}

#[tokio::test]
async fn detection_tools_a_call_without_a_name_returns_none() {
    let hook = hook(10, Vec::new());
    let continuation = ScriptedContinuation::new(Vec::new());
    let response =
        dict(r#"{"stop_reason":"tool_use","content":[{"type":"tool_use","id":"t1","input":{}}]}"#);
    let result = hook
        .post_call_success(&request(), response, &continuation)
        .await
        .unwrap();
    assert_eq!(result, None);
}

#[rstest]
#[case::code_tool("litellm_code_execution")]
#[case::skill_tool("litellm_skill_abc")]
#[case::bare_skill_prefix("litellm_skill_")]
#[tokio::test]
async fn detection_tools_executable_names_run_the_loop_and_return_its_response(#[case] name: &str) {
    let (logs, _guard) = capture();
    let hook = hook(10, vec![outcome("ran")]);
    let continuation = ScriptedContinuation::new(vec![Ok(Some(final_answer("last")))]);
    let response = tool_use("first", "t1", name, r#"{"code":"print(1)"}"#);
    let result = hook
        .post_call_success(&request(), response, &continuation)
        .await
        .unwrap()
        .expect("an executable tool call runs the loop");
    assert_eq!(marker(&result), "last");
    assert_eq!(hook.executor.codes().len(), 1);
    assert_eq!(continuation.requests().len(), 1);
    assert!(
        logs.entries()
            .contains(&(Level::DEBUG, DETECTED.to_owned()))
    );
}

#[tokio::test]
async fn detection_tools_one_executable_call_among_others_runs_the_loop() {
    let hook = hook(10, vec![outcome("ran")]);
    let continuation = ScriptedContinuation::new(vec![Ok(Some(final_answer("last")))]);
    let response = dict(
        r#"{"marker":"first","stop_reason":"tool_use","content":[
            {"type":"tool_use","id":"t1","name":"other_tool","input":{}},
            {"type":"tool_use","id":"t2","name":"litellm_code_execution","input":{"code":"print(1)"}}
        ]}"#,
    );
    let result = hook
        .post_call_success(&request(), response, &continuation)
        .await
        .unwrap()
        .expect("one executable call is enough");
    assert_eq!(marker(&result), "last");
    assert_eq!(hook.executor.codes(), vec!["print(1)".to_owned()]);
}

#[tokio::test]
async fn detection_tools_typed_response_blocks_are_detected() {
    let hook = hook(10, vec![outcome("ran")]);
    let continuation = ScriptedContinuation::new(vec![Ok(Some(final_answer("last")))]);
    let response = object_response(
        vec![tool_block(
            "t1",
            "litellm_code_execution",
            r#"{"code":"print(2)"}"#,
        )],
        "tool_use",
    );
    let result = hook
        .post_call_success(&request(), response, &continuation)
        .await
        .unwrap();
    assert_eq!(result.map(|reply| marker(&reply)), Some("last".to_owned()));
    assert_eq!(hook.executor.codes(), vec!["print(2)".to_owned()]);
}

#[tokio::test]
async fn detection_tools_malformed_openai_arguments_propagate_the_parse_error() {
    let hook = hook(10, Vec::new());
    let continuation = ScriptedContinuation::new(Vec::new());
    let error = hook
        .post_call_success(&request(), chat_tool_call("{not json"), &continuation)
        .await
        .unwrap_err();
    let SkillsLoopError::ToolCalls(source) = error;
    assert!(source.to_string().contains("litellm_code_execution"));
    assert!(continuation.requests().is_empty());
}

#[tokio::test]
async fn detection_tools_openai_tool_calls_are_detected_through_the_choices() {
    let hook = hook(10, vec![outcome("ran")]);
    let continuation = ScriptedContinuation::new(vec![Ok(Some(final_answer("last")))]);
    let result = hook
        .post_call_success(
            &request(),
            chat_tool_call(r#"{"code":"print(3)"}"#),
            &continuation,
        )
        .await
        .unwrap();
    assert_eq!(result.map(|reply| marker(&reply)), Some("last".to_owned()));
    assert_eq!(hook.executor.codes(), vec!["print(3)".to_owned()]);
}
