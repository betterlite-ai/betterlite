use rstest::rstest;

use super::fixtures::{
    dict, final_answer, hook, json, outcome, outcome_with_file, results_of, run_loop, tool_use,
};

#[rstest]
#[case::code_present(r#"{"code":"print(1)"}"#, "print(1)")]
#[case::code_absent("{}", "")]
#[case::code_a_number(r#"{"code":5}"#, "")]
#[case::code_null(r#"{"code":null}"#, "")]
#[tokio::test]
async fn loop_dispatch_code_tool_runs_the_code_string_or_an_empty_one(
    #[case] input: &str,
    #[case] expected_code: &str,
) {
    let hook = hook(10, vec![outcome("ran")]);
    let response = tool_use("first", "t1", "litellm_code_execution", input);
    let (result, continuation) =
        run_loop(&hook, response, vec![Ok(Some(final_answer("last")))]).await;
    result.unwrap();
    assert_eq!(hook.executor.codes(), vec![expected_code.to_owned()]);
    assert_eq!(
        results_of(&continuation, 0),
        json(r#"[{"type":"tool_result","tool_use_id":"t1","content":"ran"}]"#)
    );
}

#[tokio::test]
async fn loop_dispatch_skill_tool_runs_the_generated_skill_code() {
    let hook = hook(10, vec![outcome("built")]);
    let response = tool_use("first", "t1", "litellm_skill_one", "{}");
    let (result, continuation) =
        run_loop(&hook, response, vec![Ok(Some(final_answer("last")))]).await;
    result.unwrap();
    let codes = hook.executor.codes();
    assert_eq!(codes.len(), 1);
    assert!(codes[0].contains("from core.gif_builder import *"));
    assert_eq!(
        results_of(&continuation, 0),
        json(r#"[{"type":"tool_result","tool_use_id":"t1","content":"built"}]"#)
    );
}

#[rstest]
#[case::named("other_tool", "Tool 'other_tool' not handled")]
#[case::code_tool_with_suffix(
    "litellm_code_execution_x",
    "Tool 'litellm_code_execution_x' not handled"
)]
#[case::skill_prefix_without_underscore("litellm_skill", "Tool 'litellm_skill' not handled")]
#[tokio::test]
async fn loop_dispatch_other_tools_get_a_not_handled_result(
    #[case] name: &str,
    #[case] expected: &str,
) {
    let hook = hook(10, Vec::new());
    let response = tool_use("first", "t1", name, "{}");
    let (result, continuation) =
        run_loop(&hook, response, vec![Ok(Some(final_answer("last")))]).await;
    result.unwrap();
    assert!(hook.executor.codes().is_empty());
    assert_eq!(
        results_of(&continuation, 0),
        json(&format!(
            r#"[{{"type":"tool_result","tool_use_id":"t1","content":"{expected}"}}]"#
        ))
    );
}

#[tokio::test]
async fn loop_dispatch_a_call_without_a_name_is_not_handled_and_keeps_a_null_id() {
    let hook = hook(10, Vec::new());
    let response = dict(r#"{"stop_reason":"tool_use","content":[{"type":"tool_use","input":{}}]}"#);
    let (result, continuation) =
        run_loop(&hook, response, vec![Ok(Some(final_answer("last")))]).await;
    result.unwrap();
    assert_eq!(
        results_of(&continuation, 0),
        json(r#"[{"type":"tool_result","tool_use_id":null,"content":"Tool '' not handled"}]"#)
    );
}

#[tokio::test]
async fn loop_dispatch_several_calls_answer_in_order_with_their_own_ids() {
    let hook = hook(10, vec![outcome("first output"), outcome("second output")]);
    let response = dict(
        r#"{"stop_reason":"tool_use","content":[
            {"type":"tool_use","id":"t1","name":"litellm_code_execution","input":{"code":"a"}},
            {"type":"tool_use","id":"t2","name":"other_tool","input":{}},
            {"type":"tool_use","id":"t3","name":"litellm_code_execution","input":{"code":"b"}}
        ]}"#,
    );
    let (result, continuation) =
        run_loop(&hook, response, vec![Ok(Some(final_answer("last")))]).await;
    result.unwrap();
    assert_eq!(hook.executor.codes(), vec!["a".to_owned(), "b".to_owned()]);
    assert_eq!(
        results_of(&continuation, 0),
        json(
            r#"[
                {"type":"tool_result","tool_use_id":"t1","content":"first output"},
                {"type":"tool_result","tool_use_id":"t2","content":"Tool 'other_tool' not handled"},
                {"type":"tool_result","tool_use_id":"t3","content":"second output"}
            ]"#
        )
    );
}

#[tokio::test]
async fn loop_dispatch_sandbox_files_are_listed_in_the_tool_result() {
    let hook = hook(10, vec![outcome_with_file("drew", "chart.png", "aGVsbG8=")]);
    let response = tool_use(
        "first",
        "t1",
        "litellm_code_execution",
        r#"{"code":"plot()"}"#,
    );
    let (result, continuation) =
        run_loop(&hook, response, vec![Ok(Some(final_answer("last")))]).await;
    result.unwrap();
    assert_eq!(
        results_of(&continuation, 0),
        json(
            r#"[{"type":"tool_result","tool_use_id":"t1","content":"drew\n\nGenerated file: chart.png"}]"#
        )
    );
}
