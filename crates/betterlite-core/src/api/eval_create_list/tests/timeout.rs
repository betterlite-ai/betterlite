use std::time::Duration;

use betterlite_error::error::ErrorClass;

use crate::api::eval_create_list::call::{EvalCreation, EvalListing};
use crate::api::eval_create_list::error::EvalsCreateListError;
use crate::api::eval_create_list::tests::support::{
    Calls, Harness, create_call, eval, evals_page, list_call,
};

#[tokio::test]
async fn create_eval_defaults_to_the_module_request_timeout() {
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
    assert_eq!(creates[0].timeout, Duration::from_secs(6000));
}

#[tokio::test]
async fn create_eval_keeps_an_explicit_timeout() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.timeout = Some(Duration::from_secs(5));
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
    assert_eq!(creates[0].timeout, Duration::from_secs(5));
}

#[tokio::test]
async fn create_eval_zero_timeout_falls_back() {
    let mut harness = Harness::new();
    harness.core.request_timeout_seconds = 30.0_f64;
    let mut calls = Calls::empty();
    calls.timeout = Some(Duration::ZERO);
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
    assert_eq!(creates[0].timeout, Duration::from_secs(30));
}

#[tokio::test]
async fn list_evals_defaults_to_the_module_request_timeout() {
    let harness = Harness::new();
    let calls = Calls::empty();
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
    assert_eq!(lists[0].timeout, Duration::from_secs(6000));
}

#[tokio::test]
async fn list_evals_keeps_an_explicit_timeout() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.timeout = Some(Duration::from_secs(5));
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
    assert_eq!(lists[0].timeout, Duration::from_secs(5));
}

#[tokio::test]
async fn list_evals_zero_timeout_falls_back() {
    let mut harness = Harness::new();
    harness.core.request_timeout_seconds = 30.0_f64;
    let mut calls = Calls::empty();
    calls.timeout = Some(Duration::ZERO);
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
    assert_eq!(lists[0].timeout, Duration::from_secs(30));
}

#[tokio::test]
async fn an_unusable_request_timeout_is_reported_before_transport() {
    let mut harness = Harness::new();
    harness.core.request_timeout_seconds = -1.0_f64;
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
        .expect_err("an unusable timeout fails the create");
    assert!(matches!(
        &failure.error,
        EvalsCreateListError::Timeout { .. }
    ));
    assert_eq!(
        failure.error.to_string(),
        "the request timeout of -1 seconds is not a usable duration"
    );
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert!(harness.handler.creates().is_empty());
}
