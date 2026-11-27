use crate::api::eval_create_list::call::{EvalCreation, EvalListing};
use crate::api::eval_create_list::tests::support::{
    Calls, Harness, create_call, eval, evals_page, list_call,
};

#[tokio::test]
async fn create_eval_returns_a_ready_answer() {
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
    let returned = harness
        .service()
        .create_eval(creation)
        .await
        .expect("the create succeeds");
    assert_eq!(returned.id, "eval_1");
    assert_eq!(harness.handler.completed(), 1);
}

#[tokio::test]
async fn acreate_eval_waits_for_a_suspended_handler() {
    let harness = Harness::new();
    harness.handler.suspend(5);
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
    let returned = harness
        .service()
        .acreate_eval(creation)
        .await
        .expect("the create succeeds");
    assert_eq!(returned.id, "eval_1");
    assert_eq!(harness.handler.completed(), 1);
}

#[tokio::test]
async fn list_evals_returns_a_ready_answer() {
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
    let returned = harness
        .service()
        .list_evals(listing)
        .await
        .expect("the list succeeds");
    assert_eq!(returned.object, "list");
    assert_eq!(returned.data.len(), 1);
    assert_eq!(returned.data[0].id, "eval_1");
    assert_eq!(harness.handler.completed(), 1);
}

#[tokio::test]
async fn alist_evals_waits_for_a_suspended_handler() {
    let harness = Harness::new();
    harness.handler.suspend(5);
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
    let returned = harness
        .service()
        .alist_evals(listing)
        .await
        .expect("the list succeeds");
    assert_eq!(returned.data.len(), 1);
    assert_eq!(harness.handler.completed(), 1);
}
