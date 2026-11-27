use betterlite_openai::evals::types::{EvalOrder, EvalOrderBy, ListEvalsParams};

use crate::api::eval_create_list::call::EvalListing;
use crate::api::eval_create_list::tests::support::{
    Calls, Harness, evals_page, json, list_call, map, object,
};

#[tokio::test]
async fn list_evals_sends_only_the_explicit_pagination_parameters() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let mut call = list_call(&calls);
    call.limit = Some(2);
    call.after = Some("eval_0");
    call.order = Some(EvalOrder::Desc);
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
    assert_eq!(
        lists[0].query,
        object(r#"{"limit":2,"after":"eval_0","order":"desc"}"#)
    );
    assert_eq!(lists[0].url, "https://api.openai.com/v1/evals");
    let recorded = harness.recorded();
    assert_eq!(
        recorded.lists[0].params,
        ListEvalsParams {
            limit: Some(2),
            after: Some("eval_0".to_owned()),
            before: None,
            order: Some(EvalOrder::Desc),
            order_by: None,
        }
    );
}

#[tokio::test]
async fn list_evals_extra_query_overrides_the_explicit_parameters() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_query = map(&[
        ("limit", json("7")),
        ("after", json("\"over\"")),
        ("before", json("\"b\"")),
        ("order", json("\"desc\"")),
        ("order_by", json("\"updated_at\"")),
    ]);
    let mut call = list_call(&calls);
    call.limit = Some(2);
    call.after = Some("explicit");
    call.order = Some(EvalOrder::Asc);
    call.order_by = Some(EvalOrderBy::CreatedAt);
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
    assert_eq!(
        lists[0].query,
        object(r#"{"limit":7,"after":"over","before":"b","order":"desc","order_by":"updated_at"}"#)
    );
}

#[tokio::test]
async fn list_evals_extra_query_null_drops_a_parameter() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_query = map(&[("limit", json("null")), ("after", json("null"))]);
    let mut call = list_call(&calls);
    call.limit = Some(2);
    call.after = Some("eval_0");
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
    assert_eq!(lists[0].query, object("{}"));
    let recorded = harness.recorded();
    assert_eq!(recorded.lists[0].params.limit, None);
    assert_eq!(recorded.lists[0].params.after, None);
}

#[tokio::test]
async fn list_evals_ignores_unknown_extra_query_keys() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_query = map(&[("model", json("\"m\"")), ("seed", json("3"))]);
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
    assert_eq!(lists[0].query, object("{}"));
    let recorded = harness.recorded();
    assert_eq!(
        recorded.lists[0].params,
        ListEvalsParams {
            limit: None,
            after: None,
            before: None,
            order: None,
            order_by: None,
        }
    );
}

#[tokio::test]
async fn list_evals_zero_and_empty_filters_are_dropped() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let mut call = list_call(&calls);
    call.limit = Some(0);
    call.after = Some("");
    call.before = Some("b");
    call.order_by = Some(EvalOrderBy::CreatedAt);
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
    assert_eq!(
        lists[0].query,
        object(r#"{"before":"b","order_by":"created_at"}"#)
    );
    let recorded = harness.recorded();
    assert_eq!(recorded.lists[0].params.limit, Some(0));
    assert_eq!(recorded.lists[0].params.after, Some(String::new()));
}
