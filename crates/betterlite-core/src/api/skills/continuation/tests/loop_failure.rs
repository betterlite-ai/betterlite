use tracing::Level;

use super::fixtures::{
    ScriptedContinuation, ScriptedFailure, capture, chat_tool_call, generated_names, hook, marker,
    outcome, outcome_with_file, request, run_loop, skill_files, tool_use,
};
use crate::api::skills::continuation::ContinuationLoop as _;
use crate::api::skills::continuation::error::SkillsLoopError;

#[tokio::test]
async fn loop_failure_none_after_round_two_returns_the_original_with_all_files() {
    let (logs, _guard) = capture();
    let hook = hook(
        10,
        vec![
            outcome_with_file("one", "a.png", "aGk="),
            outcome_with_file("two", "b.png", "aGk="),
        ],
    );
    let first = tool_use("first", "t1", "litellm_code_execution", "{}");
    let second = tool_use("second", "t2", "litellm_code_execution", "{}");
    let (outcome, continuation) = run_loop(&hook, first, vec![Ok(Some(second)), Ok(None)]).await;
    let result = outcome.unwrap();
    assert_eq!(marker(&result), "first");
    assert_eq!(generated_names(&result), vec!["a.png", "b.png"]);
    assert_eq!(continuation.requests().len(), 2);
    assert!(logs.entries().contains(&(
        Level::ERROR,
        "SkillsInjectionHook: LLM call returned None".to_owned()
    )));
}

#[tokio::test]
async fn loop_failure_a_continuation_error_returns_the_original_and_logs_it() {
    let (logs, _guard) = capture();
    let hook = hook(10, vec![outcome_with_file("one", "a.png", "aGk=")]);
    let first = tool_use("first", "t1", "litellm_code_execution", "{}");
    let (outcome, _continuation) = run_loop(&hook, first, vec![Err(ScriptedFailure)]).await;
    let result = outcome.unwrap();
    assert_eq!(marker(&result), "first");
    assert_eq!(generated_names(&result), vec!["a.png"]);
    assert!(logs.entries().contains(&(
        Level::ERROR,
        "SkillsInjectionHook: LLM call failed: the scripted continuation failed".to_owned()
    )));
}

#[tokio::test]
async fn loop_failure_without_any_file_the_original_comes_back_unchanged() {
    let hook = hook(10, vec![outcome("ran")]);
    let first = tool_use("first", "t1", "litellm_code_execution", "{}");
    let (result, _continuation) = run_loop(&hook, first.clone(), vec![Ok(None)]).await;
    assert_eq!(result.unwrap(), first);
}

#[tokio::test]
async fn loop_failure_an_unreadable_tool_call_in_a_continuation_reply_is_an_error() {
    let hook = hook(10, vec![outcome("ran")]);
    let first = tool_use("first", "t1", "litellm_code_execution", "{}");
    let (result, continuation) =
        run_loop(&hook, first, vec![Ok(Some(chat_tool_call("{not json")))]).await;
    assert!(matches!(result, Err(SkillsLoopError::ToolCalls(_))));
    assert_eq!(continuation.requests().len(), 1);
}

#[tokio::test]
async fn loop_failure_an_unreadable_first_response_is_an_error_before_any_call() {
    let hook = hook(10, vec![outcome("ran")]);
    let continuation = ScriptedContinuation::new(Vec::new());
    let files = skill_files(&[("a.py", "a")]);
    let result = hook
        .execute_messages_loop(
            &request(),
            chat_tool_call("{not json"),
            &files,
            &continuation,
        )
        .await;
    assert!(matches!(result, Err(SkillsLoopError::ToolCalls(_))));
    assert!(hook.executor.codes().is_empty());
    assert!(continuation.requests().is_empty());
}
