use crate::api::eval_create_list::call::{EvalCreation, EvalListing};
use crate::api::eval_create_list::tests::support::{
    Calls, Harness, create_call, eval, evals_page, json, list_call, map,
};

#[tokio::test]
async fn create_eval_uses_the_kwargs_api_base() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.kwargs = map(&[("api_base", json("\"http://upstream.test\""))]);
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
    assert_eq!(creates[0].url, "http://upstream.test/v1/evals");
    let recorded = harness.recorded();
    assert_eq!(
        recorded.creates[0]
            .litellm_params
            .get("api_base")
            .and_then(|value| value.as_str()),
        Some("http://upstream.test")
    );
}

#[tokio::test]
async fn create_eval_empty_api_base_falls_back_to_the_default() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.kwargs = map(&[("api_base", json("\"\""))]);
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
    assert_eq!(creates[0].url, "https://api.openai.com/v1/evals");
}

#[tokio::test]
async fn list_evals_uses_the_default_base() {
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
    assert_eq!(lists[0].url, "https://api.openai.com/v1/evals");
}

#[tokio::test]
async fn list_evals_uses_the_kwargs_api_base() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.kwargs = map(&[("api_base", json("\"http://upstream.test\""))]);
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
    assert_eq!(lists[0].url, "http://upstream.test/v1/evals");
    let recorded = harness.recorded();
    assert_eq!(
        recorded.lists[0]
            .litellm_params
            .get("api_base")
            .and_then(|value| value.as_str()),
        Some("http://upstream.test")
    );
}

#[tokio::test]
async fn list_evals_empty_api_base_falls_back_to_the_default() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.kwargs = map(&[("api_base", json("\"\""))]);
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
    assert_eq!(lists[0].url, "https://api.openai.com/v1/evals");
}
