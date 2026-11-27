use betterlite_error::error::ErrorClass;
use betterlite_openai::evals::error::EvalsError;

use crate::api::eval_get_update::call::{EvalRetrieval, EvalUpdate};
use crate::api::eval_get_update::error::EvalsGetUpdateError;
use crate::api::eval_get_update::tests::support::{
    Calls, Harness, eval, get_call, json, map, object, provider_failure, update_call,
};
use crate::api::eval_http::error::EvalHttpError;

#[tokio::test]
async fn get_eval_maps_a_provider_not_found() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let call = get_call(&calls, "eval_1");
    let mut state = harness.logging_state("aget_eval");
    let env = harness.env.pre_post();
    harness
        .handler
        .answer_get(Err(provider_failure(404, "No such eval", None)));
    let retrieval = EvalRetrieval {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let failure = harness
        .service()
        .get_eval(retrieval)
        .await
        .expect_err("the provider error fails the get");
    assert!(matches!(
        &failure.error,
        EvalsGetUpdateError::Handler(EvalHttpError::Api(EvalsError::Provider { .. }))
    ));
    assert!(matches!(failure.class, ErrorClass::NotFound(_)));
    assert_eq!(failure.error.to_string(), "No such eval");
    assert_eq!(harness.handler.gets().len(), 1);
}

#[tokio::test]
async fn update_eval_maps_a_provider_rate_limit() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let call = update_call(&calls, "eval_1", None, None);
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    harness
        .handler
        .answer_update(Err(provider_failure(429, "slow down", None)));
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let failure = harness
        .service()
        .update_eval(update)
        .await
        .expect_err("the provider error fails the update");
    assert!(matches!(
        &failure.error,
        EvalsGetUpdateError::Handler(EvalHttpError::Api(EvalsError::Provider { .. }))
    ));
    assert!(matches!(failure.class, ErrorClass::RateLimit(_)));
    assert_eq!(harness.handler.updates().len(), 1);
}

#[tokio::test]
async fn get_eval_maps_a_provider_bad_request_body() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let call = get_call(&calls, "eval_1");
    let mut state = harness.logging_state("aget_eval");
    let env = harness.env.pre_post();
    let body = object(
        r#"{"message":"unknown eval","type":"invalid_request_error","param":"eval_id","code":"invalid"}"#,
    );
    harness
        .handler
        .answer_get(Err(provider_failure(400, "unknown eval", Some(body))));
    let retrieval = EvalRetrieval {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let failure = harness
        .service()
        .get_eval(retrieval)
        .await
        .expect_err("the provider error fails the get");
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!(
            "the provider 400 maps to a bad request, not {:?}",
            failure.class
        );
    };
    assert_eq!(details.body_type.as_deref(), Some("invalid_request_error"));
    assert_eq!(details.body_param.as_deref(), Some("eval_id"));
    assert_eq!(details.body_code.as_deref(), Some("invalid"));
    assert_eq!(details.status.0, 400);
}

#[tokio::test]
async fn update_eval_maps_a_provider_internal_error() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let call = update_call(&calls, "eval_1", None, None);
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    harness
        .handler
        .answer_update(Err(provider_failure(500, "the provider broke", None)));
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let failure = harness
        .service()
        .update_eval(update)
        .await
        .expect_err("the provider error fails the update");
    assert!(matches!(failure.class, ErrorClass::InternalServer(_)));
}

#[tokio::test]
async fn update_eval_reports_a_wrong_typed_name_in_extra_body() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_body = map(&[("name", json("42"))]);
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
        .expect_err("a wrong-typed extra_body value fails the update");
    assert!(matches!(
        &failure.error,
        EvalsGetUpdateError::FieldType { .. }
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
    assert!(harness.handler.updates().is_empty());
    assert!(harness.recorded().validated.is_empty());
}

#[tokio::test]
async fn update_eval_reports_a_wrong_typed_metadata_in_extra_body() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_body = map(&[("metadata", json("\"not an object\""))]);
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
        .expect_err("a wrong-typed metadata fails the update");
    assert!(matches!(
        &failure.error,
        EvalsGetUpdateError::FieldType { .. }
    ));
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!(
            "a wrong-typed value maps to a bad request, not {:?}",
            failure.class
        );
    };
    assert_eq!(details.body_param.as_deref(), Some("metadata"));
    assert_eq!(
        details.message,
        "the metadata field does not have the declared type"
    );
    assert!(harness.handler.updates().is_empty());
}
