use betterlite_openai::evals::types::{CreateEvalRequest, DataSourceConfig, GraderConfig};

use crate::api::eval_create_list::call::EvalCreation;
use crate::api::eval_create_list::tests::support::{
    Calls, Harness, create_call, eval, json, judge_grader, logs_config, map, object,
};

#[tokio::test]
async fn create_eval_builds_the_request_from_parameters() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let call = create_call(&calls, Some("my-eval"), None);
    let mut state = harness.logging_state("acreate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_create(Ok(eval("eval_1")));
    let creation = EvalCreation {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let returned = harness
        .service()
        .create_eval(creation)
        .await
        .expect("the create succeeds");
    assert_eq!(returned.id, "eval_1");
    assert_eq!(returned.object, "eval");
    let creates = harness.handler.creates();
    assert_eq!(creates.len(), 1);
    assert_eq!(
        creates[0].body,
        object(
            r#"{"data_source_config":{"type":"logs"},"testing_criteria":[{"type":"llm_as_judge","model":"gpt-4o"}],"name":"my-eval"}"#
        )
    );
    assert_eq!(creates[0].url, "https://api.openai.com/v1/evals");
    let recorded = harness.recorded();
    assert_eq!(
        recorded.creates[0].request,
        CreateEvalRequest {
            name: Some("my-eval".to_owned()),
            data_source_config: logs_config(),
            testing_criteria: vec![judge_grader()],
            metadata: None,
        }
    );
}

#[tokio::test]
async fn create_eval_leaves_the_user_metadata_out() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let metadata = map(&[("team", json("\"core\""))]);
    let call = create_call(&calls, None, Some(&metadata));
    assert!(call.metadata.is_some());
    let mut state = harness.logging_state("acreate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_create(Ok(eval("eval_1")));
    let creation = EvalCreation {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .create_eval(creation)
        .await
        .expect("the create succeeds");
    let creates = harness.handler.creates();
    assert!(!creates[0].body.contains_key("metadata"));
    let recorded = harness.recorded();
    assert_eq!(recorded.creates[0].request.metadata, None);
    assert_eq!(recorded.creates[0].request.name, None);
}

#[tokio::test]
async fn create_eval_applies_extra_body_updates() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_body = map(&[
        ("name", json("\"renamed\"")),
        ("metadata", json("{\"team\":\"core\"}")),
        ("seed", json("42")),
        ("dropped", json("null")),
    ]);
    let call = create_call(&calls, Some("original"), None);
    let mut state = harness.logging_state("acreate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_create(Ok(eval("eval_1")));
    let creation = EvalCreation {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .create_eval(creation)
        .await
        .expect("the create succeeds");
    let creates = harness.handler.creates();
    assert_eq!(
        creates[0].body,
        object(
            r#"{"data_source_config":{"type":"logs"},"testing_criteria":[{"type":"llm_as_judge","model":"gpt-4o"}],"name":"renamed","metadata":{"team":"core"},"seed":42}"#
        )
    );
    assert!(!creates[0].body.contains_key("dropped"));
    let recorded = harness.recorded();
    assert_eq!(recorded.creates[0].request.name, Some("renamed".to_owned()));
    assert_eq!(
        recorded.creates[0].request.metadata,
        Some(object(r#"{"team":"core"}"#))
    );
}

#[tokio::test]
async fn create_eval_extra_body_null_clears_the_name() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_body = map(&[("name", json("null")), ("dropped", json("null"))]);
    let call = create_call(&calls, Some("original"), None);
    let mut state = harness.logging_state("acreate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_create(Ok(eval("eval_1")));
    let creation = EvalCreation {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .create_eval(creation)
        .await
        .expect("the create succeeds");
    let creates = harness.handler.creates();
    assert_eq!(
        creates[0].body,
        object(
            r#"{"data_source_config":{"type":"logs"},"testing_criteria":[{"type":"llm_as_judge","model":"gpt-4o"}]}"#
        )
    );
    assert!(!creates[0].body.contains_key("name"));
    let recorded = harness.recorded();
    assert_eq!(recorded.creates[0].request.name, None);
}

#[tokio::test]
async fn create_eval_extra_body_replaces_the_data_source_config() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_body = map(&[(
        "data_source_config",
        json("{\"type\":\"stored_completions\",\"metadata\":{\"team\":\"core\"}}"),
    )]);
    let call = create_call(&calls, None, None);
    let mut state = harness.logging_state("acreate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_create(Ok(eval("eval_1")));
    let creation = EvalCreation {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .create_eval(creation)
        .await
        .expect("the create succeeds");
    let creates = harness.handler.creates();
    assert_eq!(
        creates[0].body,
        object(
            r#"{"data_source_config":{"type":"stored_completions","metadata":{"team":"core"}},"testing_criteria":[{"type":"llm_as_judge","model":"gpt-4o"}]}"#
        )
    );
    let recorded = harness.recorded();
    assert_eq!(
        recorded.creates[0].request.data_source_config,
        serde_json::from_str::<DataSourceConfig>(
            r#"{"type":"stored_completions","metadata":{"team":"core"}}"#
        )
        .expect("the data source fixture is valid")
    );
}

#[tokio::test]
async fn create_eval_extra_body_replaces_the_testing_criteria() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_body = map(&[(
        "testing_criteria",
        json("[{\"type\":\"ground_truth\",\"metric\":\"exact_match\"}]"),
    )]);
    let call = create_call(&calls, None, None);
    let mut state = harness.logging_state("acreate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_create(Ok(eval("eval_1")));
    let creation = EvalCreation {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .create_eval(creation)
        .await
        .expect("the create succeeds");
    let creates = harness.handler.creates();
    assert_eq!(
        creates[0].body,
        object(
            r#"{"data_source_config":{"type":"logs"},"testing_criteria":[{"type":"ground_truth","metric":"exact_match"}]}"#
        )
    );
    let recorded = harness.recorded();
    assert_eq!(
        recorded.creates[0].request.testing_criteria,
        serde_json::from_str::<Vec<GraderConfig>>(
            r#"[{"type":"ground_truth","metric":"exact_match"}]"#
        )
        .expect("the grader fixture is valid")
    );
}
