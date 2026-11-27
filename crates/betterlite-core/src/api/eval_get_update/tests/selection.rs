use betterlite_error::error::ErrorClass;

use crate::api::eval_get_update::call::{EvalRetrieval, EvalUpdate};
use crate::api::eval_get_update::error::EvalsGetUpdateError;
use crate::api::eval_get_update::tests::support::{Calls, Harness, eval, get_call, update_call};

#[tokio::test]
async fn get_eval_defaults_to_openai() {
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
    let gets = harness.handler.gets();
    assert_eq!(gets[0].provider, Some("openai".to_owned()));
    assert_eq!(harness.recorded().gets.len(), 1);
}

#[tokio::test]
async fn aget_eval_defaults_to_openai() {
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
        .aget_eval(retrieval)
        .await
        .expect("the get succeeds");
    assert_eq!(
        harness.handler.gets()[0].provider,
        Some("openai".to_owned())
    );
}

#[tokio::test]
async fn update_eval_defaults_to_openai() {
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
    assert_eq!(updates[0].provider, Some("openai".to_owned()));
    assert_eq!(harness.recorded().updates.len(), 1);
}

#[tokio::test]
async fn get_eval_rejects_a_provider_without_an_evals_config() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.provider = Some("anthropic".to_owned());
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
        .expect_err("an unsupported provider fails the get");
    assert!(matches!(
        &failure.error,
        EvalsGetUpdateError::GetUnsupported { .. }
    ));
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert_eq!(
        failure.error.to_string(),
        "GET eval is not supported for anthropic"
    );
    assert!(harness.handler.gets().is_empty());
    assert!(harness.recorded().validated.is_empty());
}

#[tokio::test]
async fn aupdate_eval_rejects_a_provider_without_an_evals_config() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.provider = Some("anthropic".to_owned());
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
        .aupdate_eval(update)
        .await
        .expect_err("an unsupported provider fails the update");
    assert!(matches!(
        &failure.error,
        EvalsGetUpdateError::UpdateUnsupported { .. }
    ));
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert_eq!(
        failure.error.to_string(),
        "UPDATE eval is not supported for anthropic"
    );
    assert!(harness.handler.updates().is_empty());
    assert!(harness.recorded().validated.is_empty());
}

#[tokio::test]
async fn get_eval_rejects_an_unknown_provider_name() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.provider = Some("nonsense".to_owned());
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
        .expect_err("an unknown provider fails the get");
    assert!(matches!(
        &failure.error,
        EvalsGetUpdateError::UnknownProvider { .. }
    ));
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert_eq!(
        failure.error.to_string(),
        "\"nonsense\" is not a provider betterlite knows"
    );
    assert!(harness.handler.gets().is_empty());
}

#[tokio::test]
async fn update_eval_rejects_an_unknown_provider_name() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.provider = Some("nonsense".to_owned());
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
        .expect_err("an unknown provider fails the update");
    assert!(matches!(
        &failure.error,
        EvalsGetUpdateError::UnknownProvider { .. }
    ));
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert!(harness.handler.updates().is_empty());
}

#[tokio::test]
async fn get_eval_does_not_fall_back_when_the_config_is_missing() {
    let harness = Harness::without_config();
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
        .expect_err("a missing config fails the get");
    assert_eq!(
        failure.error.to_string(),
        "GET eval is not supported for openai"
    );
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert!(harness.handler.gets().is_empty());
}

#[tokio::test]
async fn update_eval_does_not_fall_back_when_the_config_is_missing() {
    let harness = Harness::without_config();
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
        .expect_err("a missing config fails the update");
    assert_eq!(
        failure.error.to_string(),
        "UPDATE eval is not supported for openai"
    );
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert!(harness.handler.updates().is_empty());
}
