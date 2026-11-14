use rstest::rstest;
use tracing::Level;

use super::fixtures::{
    capture, dict, final_answer, generated_names, hook, marker, outcome_with_file, run_loop,
    tool_use,
};

#[rstest]
#[case::end_turn(r#""stop_reason":"end_turn","#)]
#[case::max_tokens(r#""stop_reason":"max_tokens","#)]
#[case::stop_sequence(r#""stop_reason":"stop_sequence","#)]
#[case::upper_case_tool_use(r#""stop_reason":"TOOL_USE","#)]
#[case::null(r#""stop_reason":null,"#)]
#[case::absent("")]
#[tokio::test]
async fn loop_termination_any_stop_reason_but_tool_use_finishes_the_first_round(
    #[case] stop_reason: &str,
) {
    let (logs, _guard) = capture();
    let hook = hook(10, vec![outcome_with_file("ran", "never.png", "aGk=")]);
    let response = dict(&format!(
        r#"{{{stop_reason}"marker":"first","content":[{{"type":"tool_use","id":"t1","name":"litellm_code_execution","input":{{"code":"x"}}}}]}}"#
    ));
    let (result, continuation) = run_loop(&hook, response.clone(), Vec::new()).await;
    assert_eq!(result.unwrap(), response);
    assert!(hook.executor.codes().is_empty());
    assert!(continuation.requests().is_empty());
    assert_eq!(
        logs.entries(),
        vec![(
            Level::DEBUG,
            "SkillsInjectionHook: Loop completed after 1 iterations, 0 files generated".to_owned()
        )]
    );
}

#[tokio::test]
async fn loop_termination_tool_use_without_tool_calls_finishes_the_first_round() {
    let (logs, _guard) = capture();
    let hook = hook(10, Vec::new());
    let response = dict(
        r#"{"marker":"first","stop_reason":"tool_use","content":[{"type":"text","text":"hm"}]}"#,
    );
    let (result, continuation) = run_loop(&hook, response.clone(), Vec::new()).await;
    assert_eq!(result.unwrap(), response);
    assert!(continuation.requests().is_empty());
    assert!(logs.messages().contains(
        &"SkillsInjectionHook: Loop completed after 1 iterations, 0 files generated".to_owned()
    ));
}

#[tokio::test]
async fn loop_termination_files_accumulate_across_rounds_and_attach_once_at_the_end() {
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
    let (outcome, continuation) = run_loop(
        &hook,
        first,
        vec![Ok(Some(second)), Ok(Some(final_answer("last")))],
    )
    .await;
    let result = outcome.unwrap();
    assert_eq!(marker(&result), "last");
    assert_eq!(generated_names(&result), vec!["a.png", "b.png"]);
    assert_eq!(continuation.requests().len(), 2);
    assert!(logs.messages().contains(
        &"SkillsInjectionHook: Loop completed after 3 iterations, 2 files generated".to_owned()
    ));
    assert!(
        logs.entries()
            .iter()
            .all(|(level, _)| *level != Level::WARN)
    );
}

#[rstest]
#[case::zero(0, 0)]
#[case::one(1, 1)]
#[case::three(3, 3)]
#[tokio::test]
async fn loop_termination_the_iteration_limit_bounds_the_rounds_and_warns(
    #[case] max_iterations: u32,
    #[case] rounds: usize,
) {
    let (logs, _guard) = capture();
    let outcomes = (1..=rounds)
        .map(|round| outcome_with_file("ran", &format!("f{round}.png"), "aGk="))
        .collect();
    let hook = hook(max_iterations, outcomes);
    let replies = (1..=rounds)
        .map(|round| {
            Ok(Some(tool_use(
                &format!("r{round}"),
                &format!("t{round}"),
                "litellm_code_execution",
                "{}",
            )))
        })
        .collect();
    let first = tool_use("first", "t0", "litellm_code_execution", "{}");
    let (outcome, continuation) = run_loop(&hook, first, replies).await;
    let result = outcome.unwrap();
    let expected_marker = if rounds == 0 {
        "first".to_owned()
    } else {
        format!("r{rounds}")
    };
    assert_eq!(marker(&result), expected_marker);
    assert_eq!(hook.executor.codes().len(), rounds);
    assert_eq!(continuation.requests().len(), rounds);
    assert_eq!(generated_names(&result).len(), rounds);
    assert!(logs.entries().contains(&(
        Level::WARN,
        format!("SkillsInjectionHook: Max iterations ({max_iterations}) reached")
    )));
}

#[tokio::test]
async fn loop_termination_a_final_reply_after_the_last_allowed_round_still_warns() {
    let (logs, _guard) = capture();
    let hook = hook(1, vec![outcome_with_file("ran", "a.png", "aGk=")]);
    let first = tool_use("first", "t1", "litellm_code_execution", "{}");
    let (outcome, _continuation) =
        run_loop(&hook, first, vec![Ok(Some(final_answer("last")))]).await;
    let result = outcome.unwrap();
    assert_eq!(marker(&result), "last");
    assert_eq!(generated_names(&result), vec!["a.png"]);
    assert!(logs.entries().contains(&(
        Level::WARN,
        "SkillsInjectionHook: Max iterations (1) reached".to_owned()
    )));
}
