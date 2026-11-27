use betterlite_models::decisions::Json;

use crate::api::eval_get_update::call::{EvalRetrieval, EvalUpdate};
use crate::api::eval_get_update::tests::support::{
    Calls, Harness, eval, get_call, json, map, update_call,
};
use crate::api::passthrough_call::logging::json_object;

#[tokio::test]
async fn get_eval_logs_the_eval_id_before_dispatch() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.kwargs = map(&[("litellm_call_id", json("\"call-9\""))]);
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
        gets[0].optional_params,
        map(&[("eval_id", json("\"eval_1\""))])
    );
    assert_eq!(gets[0].provider, Some("openai".to_owned()));
    assert_eq!(
        gets[0].litellm_params.get("litellm_call_id"),
        Some(&json("\"call-9\""))
    );
    assert_eq!(state.optional_params, gets[0].optional_params);
    assert_eq!(state.custom_llm_provider, Some("openai".to_owned()));
    assert_eq!(
        state.model_call_details.get("optional_params"),
        Some(&Json::Object(gets[0].optional_params.clone()))
    );
}

#[tokio::test]
async fn update_eval_logs_the_transformed_body_before_dispatch() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.kwargs = map(&[("litellm_call_id", json("\"call-9\""))]);
    calls.extra_body = map(&[("seed", json("7"))]);
    let call = update_call(&calls, "eval_1", Some("my-eval"), None);
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
    assert_eq!(updates[0].optional_params, json_object(&updates[0].body));
    assert_eq!(updates[0].optional_params.get("seed"), Some(&json("7")));
    assert_eq!(
        updates[0].optional_params.get("name"),
        Some(&json("\"my-eval\""))
    );
    assert_eq!(updates[0].provider, Some("openai".to_owned()));
    assert_eq!(
        updates[0].litellm_params.get("litellm_call_id"),
        Some(&json("\"call-9\""))
    );
    assert_eq!(state.optional_params, updates[0].optional_params);
    assert_eq!(state.custom_llm_provider, Some("openai".to_owned()));
    assert_eq!(
        state.model_call_details.get("optional_params"),
        Some(&Json::Object(updates[0].optional_params.clone()))
    );
}

#[tokio::test]
async fn get_eval_logs_a_null_call_id_when_kwargs_carry_none() {
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
    assert_eq!(
        gets[0].litellm_params.get("litellm_call_id"),
        Some(&Json::Null)
    );
}

#[tokio::test]
async fn update_eval_logs_a_null_call_id_when_kwargs_carry_none() {
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
        updates[0].litellm_params.get("litellm_call_id"),
        Some(&Json::Null)
    );
}
