use crate::api::eval_get_update::call::{EvalRetrieval, EvalUpdate};
use crate::api::eval_get_update::tests::support::{Calls, Harness, eval, get_call, update_call};

#[tokio::test]
async fn get_eval_returns_a_ready_answer() {
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
    let returned = harness
        .service()
        .get_eval(retrieval)
        .await
        .expect("the get succeeds");
    assert_eq!(returned.id, "eval_1");
    assert_eq!(harness.handler.completed(), 1);
}

#[tokio::test]
async fn aget_eval_waits_for_a_suspended_handler() {
    let harness = Harness::new();
    harness.handler.suspend(5);
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
    let returned = harness
        .service()
        .aget_eval(retrieval)
        .await
        .expect("the get succeeds");
    assert_eq!(returned.id, "eval_1");
    assert_eq!(harness.handler.completed(), 1);
}

#[tokio::test]
async fn update_eval_returns_a_ready_answer() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let call = update_call(&calls, "eval_1", Some("renamed"), None);
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_update(Ok(eval("eval_1")));
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let returned = harness
        .service()
        .update_eval(update)
        .await
        .expect("the update succeeds");
    assert_eq!(returned.id, "eval_1");
    assert_eq!(harness.handler.completed(), 1);
}

#[tokio::test]
async fn aupdate_eval_waits_for_a_suspended_handler() {
    let harness = Harness::new();
    harness.handler.suspend(5);
    let calls = Calls::empty();
    let call = update_call(&calls, "eval_1", Some("renamed"), None);
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_update(Ok(eval("eval_1")));
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let returned = harness
        .service()
        .aupdate_eval(update)
        .await
        .expect("the update succeeds");
    assert_eq!(returned.id, "eval_1");
    assert_eq!(harness.handler.completed(), 1);
}
