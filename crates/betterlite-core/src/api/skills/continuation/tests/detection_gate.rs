use indexmap::IndexMap;
use rstest::rstest;
use tracing::Level;

use super::fixtures::{
    ScriptedContinuation, capture, chat_tool_call, final_answer, hook, outcome, request,
    request_with, skill_files, tool_use,
};
use crate::api::skills::continuation::ContinuationLoop;
use crate::api::skills::continuation::error::SkillsLoopError;
use crate::api::skills::hook::execution::SkillFiles;

const NO_FILES_WARNING: &str = "SkillsInjectionHook: No skill files found, cannot execute code";

#[tokio::test]
async fn detection_gate_disabled_flag_returns_none_without_any_call() {
    let hook = hook(10, vec![outcome("ran")]);
    let continuation = ScriptedContinuation::new(Vec::new());
    let mut data = request();
    data.skills.code_execution_enabled = false;
    let response = tool_use(
        "first",
        "t1",
        "litellm_code_execution",
        r#"{"code":"print(1)"}"#,
    );
    let result = hook
        .post_call_success(&data, response, &continuation)
        .await
        .unwrap();
    assert_eq!(result, None);
    assert!(hook.executor.codes().is_empty());
    assert!(continuation.requests().is_empty());
}

#[tokio::test]
async fn detection_gate_disabled_flag_skips_reading_the_tool_calls() {
    let hook = hook(10, Vec::new());
    let continuation = ScriptedContinuation::new(Vec::new());
    let mut data = request();
    data.skills.code_execution_enabled = false;
    let result = hook
        .post_call_success(&data, chat_tool_call("{not json"), &continuation)
        .await;
    assert!(matches!(result, Ok(None)));
}

#[tokio::test]
async fn detection_gate_disabled_flag_logs_nothing_about_files() {
    let (logs, _guard) = capture();
    let hook = hook(10, Vec::new());
    let continuation = ScriptedContinuation::new(Vec::new());
    let mut data = request_with(IndexMap::new());
    data.skills.code_execution_enabled = false;
    hook.post_call_success(&data, final_answer("first"), &continuation)
        .await
        .unwrap();
    assert!(logs.entries().is_empty());
}

#[rstest]
#[case::no_skill_ids(IndexMap::new())]
#[case::one_skill_without_files(IndexMap::from([("litellm_skill_one".to_owned(), SkillFiles::new())]))]
#[tokio::test]
async fn detection_gate_empty_skill_files_return_none_and_warn(
    #[case] by_skill: IndexMap<String, SkillFiles>,
) {
    let (logs, _guard) = capture();
    let hook = hook(10, vec![outcome("ran")]);
    let continuation = ScriptedContinuation::new(Vec::new());
    let data = request_with(by_skill);
    let response = tool_use(
        "first",
        "t1",
        "litellm_code_execution",
        r#"{"code":"print(1)"}"#,
    );
    let result = hook
        .post_call_success(&data, response, &continuation)
        .await
        .unwrap();
    assert_eq!(result, None);
    assert_eq!(
        logs.entries(),
        vec![(Level::WARN, NO_FILES_WARNING.to_owned())]
    );
    assert!(hook.executor.codes().is_empty());
    assert!(continuation.requests().is_empty());
}

#[tokio::test]
async fn detection_gate_skill_files_are_merged_in_request_order_with_later_values_winning() {
    let mut by_skill = IndexMap::new();
    by_skill.insert(
        "litellm_skill_a".to_owned(),
        skill_files(&[("a.py", "a"), ("shared.txt", "old")]),
    );
    by_skill.insert(
        "litellm_skill_b".to_owned(),
        skill_files(&[("shared.txt", "new"), ("b.py", "b")]),
    );
    let hook = hook(10, vec![outcome("ran")]);
    let continuation = ScriptedContinuation::new(vec![Ok(Some(final_answer("last")))]);
    let response = tool_use(
        "first",
        "t1",
        "litellm_code_execution",
        r#"{"code":"print(1)"}"#,
    );
    hook.post_call_success(&request_with(by_skill), response, &continuation)
        .await
        .unwrap();
    let calls = hook.executor.calls.lock().unwrap().clone();
    let merged: Vec<(&str, &[u8])> = calls[0]
        .1
        .iter()
        .map(|(path, content)| (path.as_str(), content.as_slice()))
        .collect();
    assert_eq!(
        merged,
        vec![
            ("a.py", b"a".as_slice()),
            ("shared.txt", b"new".as_slice()),
            ("b.py", b"b".as_slice()),
        ]
    );
}

#[tokio::test]
async fn detection_gate_files_of_a_skill_without_tool_call_still_gate_the_run() {
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

#[tokio::test]
async fn detection_gate_enabled_flag_with_files_reads_malformed_arguments() {
    let hook = hook(10, Vec::new());
    let continuation = ScriptedContinuation::new(Vec::new());
    let result = hook
        .post_call_success(&request(), chat_tool_call("{not json"), &continuation)
        .await;
    assert!(matches!(result, Err(SkillsLoopError::ToolCalls(_))));
}
