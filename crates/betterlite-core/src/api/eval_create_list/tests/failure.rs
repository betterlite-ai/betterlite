use betterlite_error::error::ErrorClass;
use betterlite_openai::evals::error::EvalsError;

use crate::api::eval_create_list::call::{EvalCreation, EvalListing};
use crate::api::eval_create_list::error::EvalsCreateListError;
use crate::api::eval_create_list::tests::support::{
    Calls, Harness, create_call, json, list_call, map, object, provider_failure,
};
use crate::api::eval_http::error::EvalHttpError;

#[tokio::test]
async fn create_eval_maps_a_provider_not_found() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let call = create_call(&calls, None, None);
    let mut state = harness.logging_state("acreate_eval");
    let env = harness.env.pre_post();
    harness
        .handler
        .answer_create(Err(provider_failure(404, "No such eval", None)));
    let creation = EvalCreation {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let failure = harness
        .service()
        .create_eval(creation)
        .await
        .expect_err("the provider error fails the create");
    assert!(matches!(
        &failure.error,
        EvalsCreateListError::Handler(EvalHttpError::Api(EvalsError::Provider { .. }))
    ));
    assert!(matches!(failure.class, ErrorClass::NotFound(_)));
    assert_eq!(failure.error.to_string(), "No such eval");
    assert_eq!(harness.handler.creates().len(), 1);
}

#[tokio::test]
async fn list_evals_maps_a_provider_rate_limit() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let call = list_call(&calls);
    let mut state = harness.logging_state("alist_evals");
    let env = harness.env.pre_post();
    harness
        .handler
        .answer_list(Err(provider_failure(429, "slow down", None)));
    let listing = EvalListing {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let failure = harness
        .service()
        .list_evals(listing)
        .await
        .expect_err("the provider error fails the list");
    assert!(matches!(
        &failure.error,
        EvalsCreateListError::Handler(EvalHttpError::Api(EvalsError::Provider { .. }))
    ));
    assert!(matches!(failure.class, ErrorClass::RateLimit(_)));
    assert_eq!(harness.handler.lists().len(), 1);
}

#[tokio::test]
async fn create_eval_maps_a_provider_bad_request_body() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let call = create_call(&calls, None, None);
    let mut state = harness.logging_state("acreate_eval");
    let env = harness.env.pre_post();
    let body = object(
        r#"{"message":"bad criteria","type":"invalid_request_error","param":"testing_criteria","code":"invalid"}"#,
    );
    harness
        .handler
        .answer_create(Err(provider_failure(400, "bad criteria", Some(body))));
    let creation = EvalCreation {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let failure = harness
        .service()
        .create_eval(creation)
        .await
        .expect_err("the provider error fails the create");
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!(
            "the provider 400 maps to a bad request, not {:?}",
            failure.class
        );
    };
    assert_eq!(details.body_type.as_deref(), Some("invalid_request_error"));
    assert_eq!(details.body_param.as_deref(), Some("testing_criteria"));
    assert_eq!(details.body_code.as_deref(), Some("invalid"));
    assert_eq!(details.status.0, 400);
}

#[tokio::test]
async fn list_evals_maps_a_provider_internal_error() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let call = list_call(&calls);
    let mut state = harness.logging_state("alist_evals");
    let env = harness.env.pre_post();
    harness
        .handler
        .answer_list(Err(provider_failure(500, "the provider broke", None)));
    let listing = EvalListing {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let failure = harness
        .service()
        .list_evals(listing)
        .await
        .expect_err("the provider error fails the list");
    assert!(matches!(failure.class, ErrorClass::InternalServer(_)));
}

#[tokio::test]
async fn create_eval_reports_a_wrong_typed_body_value() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_body = map(&[("name", json("42"))]);
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
        .expect_err("a wrong-typed extra_body value fails the create");
    assert!(matches!(
        &failure.error,
        EvalsCreateListError::FieldType { .. }
    ));
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!(
            "a wrong-typed value maps to a bad request, not {:?}",
            failure.class
        );
    };
    assert_eq!(details.body_param.as_deref(), Some("name"));
    assert_eq!(
        details.message,
        "the name field does not have the declared type"
    );
    assert!(harness.handler.creates().is_empty());
    assert!(harness.recorded().validated.is_empty());
}

#[tokio::test]
async fn list_evals_reports_a_wrong_typed_query_value() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_query = map(&[("limit", json("\"many\""))]);
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
        .expect_err("a wrong-typed extra_query value fails the list");
    assert!(matches!(
        &failure.error,
        EvalsCreateListError::FieldType { .. }
    ));
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!(
            "a wrong-typed value maps to a bad request, not {:?}",
            failure.class
        );
    };
    assert_eq!(details.body_param.as_deref(), Some("limit"));
    assert_eq!(
        details.message,
        "the limit field does not have the declared type"
    );
    assert!(harness.handler.lists().is_empty());
    assert!(harness.recorded().validated.is_empty());
}
