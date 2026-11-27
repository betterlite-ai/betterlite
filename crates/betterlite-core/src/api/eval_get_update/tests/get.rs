use betterlite_openai::evals::types::Eval;

use crate::api::eval_get_update::call::EvalRetrieval;
use crate::api::eval_get_update::tests::support::{Calls, Harness, eval, get_call, json, map};

#[tokio::test]
async fn get_eval_transforms_the_eval_id_into_the_url() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let call = get_call(&calls, "eval/1");
    let mut state = harness.logging_state("aget_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_get(Ok(eval("eval_1")));
    let retrieval = EvalRetrieval {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let returned = harness
        .service()
        .get_eval(retrieval)
        .await
        .expect("the get succeeds");
    assert_eq!(returned.id, "eval_1");
    let gets = harness.handler.gets();
    assert_eq!(gets.len(), 1);
    assert_eq!(gets[0].url, "https://api.openai.com/v1/evals/eval%2F1");
    let recorded = harness.recorded();
    assert_eq!(recorded.gets[0].eval_id, "eval/1");
    assert_eq!(recorded.gets[0].api_base, "https://api.openai.com");
}

#[tokio::test]
async fn get_eval_returns_the_transformed_eval() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let call = get_call(&calls, "eval_7");
    let mut state = harness.logging_state("aget_eval");
    let env = harness.env.pre_post();
    let fixture: Eval = serde_json::from_str(
        r#"{"id":"eval_7","object":"eval","created_at":1700000000,"updated_at":1700000001,"name":"nightly","data_source_config":{"type":"logs"},"testing_criteria":[]}"#,
    )
    .expect("the eval fixture is valid");
    harness.handler.answer_get(Ok(fixture));
    let retrieval = EvalRetrieval {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let returned = harness
        .service()
        .get_eval(retrieval)
        .await
        .expect("the get succeeds");
    assert_eq!(returned.id, "eval_7");
    assert_eq!(returned.object, "eval");
    assert_eq!(returned.name.as_deref(), Some("nightly"));
    assert_eq!(returned.updated_at, Some(1_700_000_001));
    assert_eq!(harness.handler.completed(), 1);
}

#[tokio::test]
async fn get_eval_accepts_extra_query_without_forwarding_it() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_query = map(&[("expand", json("\"results\""))]);
    let call = get_call(&calls, "eval_1");
    let mut state = harness.logging_state("aget_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_get(Ok(eval("eval_1")));
    let retrieval = EvalRetrieval {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .get_eval(retrieval)
        .await
        .expect("the get succeeds");
    let gets = harness.handler.gets();
    assert_eq!(gets[0].url, "https://api.openai.com/v1/evals/eval_1");
    assert!(!gets[0].url.contains('?'));
    assert!(!gets[0].url.contains("expand"));
}

#[tokio::test]
async fn get_eval_uses_the_kwargs_api_base() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.kwargs = map(&[("api_base", json("\"http://upstream.test\""))]);
    let call = get_call(&calls, "eval_1");
    let mut state = harness.logging_state("aget_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_get(Ok(eval("eval_1")));
    let retrieval = EvalRetrieval {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .get_eval(retrieval)
        .await
        .expect("the get succeeds");
    let gets = harness.handler.gets();
    assert_eq!(gets[0].url, "http://upstream.test/v1/evals/eval_1");
    let recorded = harness.recorded();
    assert_eq!(recorded.gets[0].api_base, "http://upstream.test");
    assert_eq!(
        recorded.gets[0]
            .litellm_params
            .get("api_base")
            .and_then(|value| value.as_str()),
        Some("http://upstream.test")
    );
}

#[tokio::test]
async fn get_eval_uses_an_explicit_empty_api_base_as_given() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.kwargs = map(&[("api_base", json("\"\""))]);
    let call = get_call(&calls, "eval_1");
    let mut state = harness.logging_state("aget_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_get(Ok(eval("eval_1")));
    let retrieval = EvalRetrieval {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .get_eval(retrieval)
        .await
        .expect("the get succeeds");
    let gets = harness.handler.gets();
    assert_eq!(gets[0].url, "/v1/evals/eval_1");
    assert_eq!(harness.recorded().gets[0].api_base, "");
}

#[tokio::test]
async fn get_eval_null_api_base_takes_the_default() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.kwargs = map(&[("api_base", json("null"))]);
    let call = get_call(&calls, "eval_1");
    let mut state = harness.logging_state("aget_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_get(Ok(eval("eval_1")));
    let retrieval = EvalRetrieval {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .get_eval(retrieval)
        .await
        .expect("the get succeeds");
    let gets = harness.handler.gets();
    assert_eq!(gets[0].url, "https://api.openai.com/v1/evals/eval_1");
    assert_eq!(
        harness.recorded().gets[0].api_base,
        "https://api.openai.com"
    );
}
