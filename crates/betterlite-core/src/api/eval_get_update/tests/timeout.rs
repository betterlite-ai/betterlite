use std::time::Duration;

use betterlite_error::error::ErrorClass;

use crate::api::eval_get_update::call::{EvalRetrieval, EvalUpdate};
use crate::api::eval_get_update::error::EvalsGetUpdateError;
use crate::api::eval_get_update::tests::support::{Calls, Harness, eval, get_call, update_call};

#[tokio::test]
async fn get_eval_defaults_to_the_module_request_timeout() {
    let harness = Harness::new();
    let calls = Calls::empty();
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
    assert_eq!(harness.handler.gets()[0].timeout, Duration::from_mins(100));
}

#[tokio::test]
async fn get_eval_keeps_an_explicit_timeout() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.timeout = Some(Duration::from_secs(5));
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
    assert_eq!(harness.handler.gets()[0].timeout, Duration::from_secs(5));
}

#[tokio::test]
async fn get_eval_zero_timeout_falls_back() {
    let mut harness = Harness::new();
    harness.core.request_timeout_seconds = 30.0_f64;
    let mut calls = Calls::empty();
    calls.timeout = Some(Duration::ZERO);
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
    assert_eq!(harness.handler.gets()[0].timeout, Duration::from_secs(30));
}

#[tokio::test]
async fn update_eval_defaults_to_the_module_request_timeout() {
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
    assert_eq!(
        harness.handler.updates()[0].timeout,
        Duration::from_mins(100)
    );
}

#[tokio::test]
async fn update_eval_keeps_an_explicit_timeout() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.timeout = Some(Duration::from_secs(5));
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
    assert_eq!(harness.handler.updates()[0].timeout, Duration::from_secs(5));
}

#[tokio::test]
async fn update_eval_zero_timeout_falls_back() {
    let mut harness = Harness::new();
    harness.core.request_timeout_seconds = 30.0_f64;
    let mut calls = Calls::empty();
    calls.timeout = Some(Duration::ZERO);
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
    assert_eq!(
        harness.handler.updates()[0].timeout,
        Duration::from_secs(30)
    );
}

#[tokio::test]
async fn an_unusable_request_timeout_is_reported_before_transport() {
    let mut harness = Harness::new();
    harness.core.request_timeout_seconds = -1.0_f64;
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
        .expect_err("an unusable timeout fails the get");
    assert!(matches!(
        &failure.error,
        EvalsGetUpdateError::Timeout { .. }
    ));
    assert_eq!(
        failure.error.to_string(),
        "the request timeout of -1 seconds is not a usable duration"
    );
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert!(harness.handler.gets().is_empty());
}

#[tokio::test]
async fn an_unusable_request_timeout_is_reported_before_an_update() {
    let mut harness = Harness::new();
    harness.core.request_timeout_seconds = -1.0_f64;
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
        .expect_err("an unusable timeout fails the update");
    assert!(matches!(
        &failure.error,
        EvalsGetUpdateError::Timeout { .. }
    ));
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert!(harness.handler.updates().is_empty());
}
