use betterlite_error::error::ErrorClass;
use betterlite_openai::evals::error::EvalsError;

use crate::api::eval_create_list::call::{EvalCreation, EvalListing};
use crate::api::eval_create_list::error::EvalsCreateListError;
use crate::api::eval_create_list::tests::support::{
    Calls, Harness, create_call, eval, evals_page, headers, json, list_call, map,
};
use crate::api::eval_http::error::EvalHttpError;

#[tokio::test]
async fn create_eval_validates_the_environment_with_the_caller_headers() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_headers = Some(headers(&[("x-test", "yes")]));
    calls.kwargs = map(&[("api_key", json("\"sk-call\""))]);
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
        creates[0].headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-call")
    );
    assert_eq!(
        creates[0].headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        creates[0].headers.get("x-test").map(String::as_str),
        Some("yes")
    );
    let recorded = harness.recorded();
    assert_eq!(
        recorded.validated[0].headers_in,
        headers(&[("x-test", "yes")])
    );
    assert_eq!(recorded.creates[0].headers, creates[0].headers);
}

#[tokio::test]
async fn create_eval_hands_the_kwargs_to_validate_environment() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.kwargs = map(&[("api_key", json("\"sk-call\""))]);
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
        recorded.creates[0]
            .litellm_params
            .get("api_key")
            .and_then(|value| value.as_str()),
        Some("sk-call")
    );
}

#[tokio::test]
async fn create_eval_uses_the_config_api_key_when_kwargs_carry_none() {
    let harness = Harness::new();
    let calls = Calls::empty();
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
        creates[0].headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-test")
    );
    let recorded = harness.recorded();
    assert!(recorded.validated[0].headers_in.is_empty());
    assert!(recorded.validated[0].litellm_params.is_some());
}

#[tokio::test]
async fn list_evals_passes_the_resolved_headers_to_the_transform() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_headers = Some(headers(&[("x-test", "yes")]));
    let call = list_call(&calls);
    let mut state = harness.logging_state("alist_evals");
    let env = harness.env.pre_post();
    harness.handler.answer_list(Ok(evals_page()));
    let listing = EvalListing {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .list_evals(listing)
        .await
        .expect("the list succeeds");
    let lists = harness.handler.lists();
    let recorded = harness.recorded();
    assert_eq!(recorded.validated.len(), 1);
    assert_eq!(
        recorded.validated[0].headers_in,
        headers(&[("x-test", "yes")])
    );
    assert_eq!(recorded.lists[0].headers, lists[0].headers);
    assert_eq!(
        recorded.lists[0]
            .headers
            .get("Authorization")
            .map(String::as_str),
        Some("Bearer sk-test")
    );
    assert_eq!(
        recorded.lists[0].headers.get("x-test").map(String::as_str),
        Some("yes")
    );
}

#[tokio::test]
async fn create_eval_reports_a_missing_api_key_before_transport() {
    let harness = Harness::without_credentials();
    let calls = Calls::empty();
    let call = create_call(&calls, None, None);
    let mut state = harness.logging_state("acreate_eval");
    let env = harness.env.pre_post();
    let creation = EvalCreation {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let failure = harness
        .service()
        .create_eval(creation)
        .await
        .expect_err("a missing api key fails the create");
    assert!(matches!(
        &failure.error,
        EvalsCreateListError::Handler(EvalHttpError::Api(EvalsError::ApiKeyRequired))
    ));
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert_eq!(
        failure.error.to_string(),
        "OPENAI_API_KEY is required for Evals API"
    );
    assert!(harness.handler.creates().is_empty());
    assert!(harness.recorded().creates.is_empty());
}

#[tokio::test]
async fn list_evals_reports_a_missing_api_key_before_transport() {
    let harness = Harness::without_credentials();
    let calls = Calls::empty();
    let call = list_call(&calls);
    let mut state = harness.logging_state("alist_evals");
    let env = harness.env.pre_post();
    let listing = EvalListing {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let failure = harness
        .service()
        .list_evals(listing)
        .await
        .expect_err("a missing api key fails the list");
    assert!(matches!(
        &failure.error,
        EvalsCreateListError::Handler(EvalHttpError::Api(EvalsError::ApiKeyRequired))
    ));
    assert_eq!(
        failure.error.to_string(),
        "OPENAI_API_KEY is required for Evals API"
    );
    assert!(harness.handler.lists().is_empty());
    assert!(harness.recorded().lists.is_empty());
}
