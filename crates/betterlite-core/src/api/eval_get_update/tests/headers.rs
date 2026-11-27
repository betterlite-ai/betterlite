use betterlite_error::error::ErrorClass;
use betterlite_openai::evals::error::EvalsError;

use crate::api::eval_get_update::call::{EvalRetrieval, EvalUpdate};
use crate::api::eval_get_update::error::EvalsGetUpdateError;
use crate::api::eval_get_update::tests::support::{
    Calls, Harness, eval, get_call, headers, json, map, update_call,
};
use crate::api::eval_http::error::EvalHttpError;

#[tokio::test]
async fn get_eval_validates_the_environment_with_the_caller_headers() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_headers = Some(headers(&[("x-test", "yes")]));
    calls.kwargs = map(&[("api_key", json("\"sk-call\""))]);
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
    assert_eq!(
        gets[0].headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-call")
    );
    assert_eq!(
        gets[0].headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        gets[0].headers.get("x-test").map(String::as_str),
        Some("yes")
    );
    let recorded = harness.recorded();
    assert_eq!(
        recorded.validated[0].headers_in,
        headers(&[("x-test", "yes")])
    );
    assert_eq!(recorded.gets[0].headers, gets[0].headers);
}

#[tokio::test]
async fn get_eval_hands_the_kwargs_to_validate_environment() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.kwargs = map(&[("api_key", json("\"sk-call\""))]);
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
    let recorded = harness.recorded();
    let litellm_params = recorded.validated[0]
        .litellm_params
        .as_ref()
        .expect("the kwargs object reaches validate_environment");
    assert_eq!(
        litellm_params
            .get("api_key")
            .and_then(|value| value.as_str()),
        Some("sk-call")
    );
    assert_eq!(
        recorded.gets[0]
            .litellm_params
            .get("api_key")
            .and_then(|value| value.as_str()),
        Some("sk-call")
    );
}

#[tokio::test]
async fn update_eval_passes_the_resolved_headers_to_the_transform() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_headers = Some(headers(&[("x-test", "yes")]));
    let call = update_call(&calls, "eval_1", None, None);
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_update(Ok(eval("eval_1")));
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .update_eval(update)
        .await
        .expect("the update succeeds");
    let updates = harness.handler.updates();
    let recorded = harness.recorded();
    assert_eq!(
        recorded.validated[0].headers_in,
        headers(&[("x-test", "yes")])
    );
    let resolved = &recorded.updates[0].headers;
    assert_eq!(resolved, &updates[0].headers);
    assert_eq!(
        resolved.get("Authorization").map(String::as_str),
        Some("Bearer sk-test")
    );
    assert_eq!(resolved.get("x-test").map(String::as_str), Some("yes"));
}

#[tokio::test]
async fn update_eval_uses_the_config_api_key_when_kwargs_carry_none() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let call = update_call(&calls, "eval_1", None, None);
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_update(Ok(eval("eval_1")));
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .update_eval(update)
        .await
        .expect("the update succeeds");
    let updates = harness.handler.updates();
    assert_eq!(
        updates[0].headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-test")
    );
    let recorded = harness.recorded();
    assert!(recorded.validated[0].headers_in.is_empty());
    assert!(recorded.validated[0].litellm_params.is_some());
}

#[tokio::test]
async fn get_eval_reports_a_missing_api_key_before_transport() {
    let harness = Harness::without_credentials();
    let calls = Calls::empty();
    let call = get_call(&calls, "eval_1");
    let mut state = harness.logging_state("aget_eval");
    let env = harness.env.pre_post();
    let retrieval = EvalRetrieval {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let failure = harness
        .service()
        .get_eval(retrieval)
        .await
        .expect_err("a missing api key fails the get");
    assert!(matches!(
        &failure.error,
        EvalsGetUpdateError::Handler(EvalHttpError::Api(EvalsError::ApiKeyRequired))
    ));
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert_eq!(
        failure.error.to_string(),
        "OPENAI_API_KEY is required for Evals API"
    );
    assert!(harness.handler.gets().is_empty());
    assert!(harness.recorded().gets.is_empty());
}

#[tokio::test]
async fn update_eval_reports_a_missing_api_key_before_transport() {
    let harness = Harness::without_credentials();
    let calls = Calls::empty();
    let call = update_call(&calls, "eval_1", None, None);
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let failure = harness
        .service()
        .update_eval(update)
        .await
        .expect_err("a missing api key fails the update");
    assert!(matches!(
        &failure.error,
        EvalsGetUpdateError::Handler(EvalHttpError::Api(EvalsError::ApiKeyRequired))
    ));
    assert_eq!(
        failure.error.to_string(),
        "OPENAI_API_KEY is required for Evals API"
    );
    assert!(harness.handler.updates().is_empty());
    assert!(harness.recorded().updates.is_empty());
}
