use betterlite_models::decisions::Json;

use crate::api::eval_create_list::call::{EvalCreation, EvalListing};
use crate::api::eval_create_list::tests::support::{
    Calls, Harness, create_call, eval, evals_page, json, list_call, map,
};
use crate::api::passthrough_call::logging::json_object;

#[tokio::test]
async fn create_eval_logs_the_transformed_body_before_dispatch() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.kwargs = map(&[("litellm_call_id", json("\"call-9\""))]);
    calls.extra_body = map(&[("seed", json("7"))]);
    let call = create_call(&calls, Some("my-eval"), None);
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
    assert_eq!(creates[0].optional_params, json_object(&creates[0].body));
    assert_eq!(creates[0].optional_params.get("seed"), Some(&json("7")));
    assert_eq!(
        creates[0].optional_params.get("name"),
        Some(&json("\"my-eval\""))
    );
    assert_eq!(creates[0].provider, Some("openai".to_owned()));
    assert_eq!(
        creates[0].litellm_params.get("litellm_call_id"),
        Some(&json("\"call-9\""))
    );
    assert_eq!(state.optional_params, creates[0].optional_params);
    assert_eq!(state.custom_llm_provider, Some("openai".to_owned()));
    assert_eq!(
        state.model_call_details.get("optional_params"),
        Some(&Json::Object(creates[0].optional_params.clone()))
    );
}

#[tokio::test]
async fn list_evals_logs_the_query_before_dispatch() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.kwargs = map(&[("litellm_call_id", json("\"call-9\""))]);
    let mut call = list_call(&calls);
    call.limit = Some(3);
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
    assert_eq!(lists[0].optional_params, json_object(&lists[0].query));
    assert_eq!(lists[0].optional_params.get("limit"), Some(&json("3")));
    assert_eq!(lists[0].provider, Some("openai".to_owned()));
    assert_eq!(
        lists[0].litellm_params.get("litellm_call_id"),
        Some(&json("\"call-9\""))
    );
    assert_eq!(state.optional_params, lists[0].optional_params);
    assert_eq!(state.custom_llm_provider, Some("openai".to_owned()));
}

#[tokio::test]
async fn create_eval_logs_a_null_call_id_when_kwargs_carry_none() {
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
        creates[0].litellm_params.get("litellm_call_id"),
        Some(&Json::Null)
    );
}
