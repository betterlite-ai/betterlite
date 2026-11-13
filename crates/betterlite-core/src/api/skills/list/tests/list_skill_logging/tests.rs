#![cfg(test)]

use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::api::skills::fixtures::{CALL_ID, kwargs, metadata_with, object, text};
use crate::api::skills::list_fixtures::{
    Fixture, call, call_with, hosted_params, listing, logging, map, params,
};

#[tokio::test]
async fn list_skill_logging_records_the_query_parameters() {
    let fixture = Fixture::new();
    let values = kwargs();
    let request = call_with(params(), Some(5), Some("tok"), Some("custom"), &values);
    let mut state = logging();
    fixture
        .service()
        .list_skills(listing(&request, &mut state))
        .await
        .expect("the external list succeeds");
    assert_eq!(
        state.optional_params,
        map(&[
            ("limit", Json::Number(Number::from(5_i64))),
            ("page", text("tok")),
            ("source", text("custom")),
        ])
    );
    assert_eq!(state.custom_llm_provider.as_deref(), Some("anthropic"));
}

#[tokio::test]
async fn list_skill_logging_omits_absent_query_parameters() {
    let fixture = Fixture::new();
    let values = kwargs();
    let mut state = logging();
    fixture
        .service()
        .list_skills(listing(&call(&values), &mut state))
        .await
        .expect("the external list succeeds");
    assert!(state.optional_params.is_empty());
}

#[tokio::test]
async fn list_skill_logging_records_the_call_id() {
    let fixture = Fixture::new();
    let values = kwargs();
    let mut state = logging();
    fixture
        .service()
        .list_skills(listing(&call(&values), &mut state))
        .await
        .expect("the external list succeeds");
    assert_eq!(
        state.litellm_params.get("litellm_call_id"),
        Some(&text(CALL_ID))
    );
}

#[tokio::test]
async fn list_skill_logging_uses_litellm_proxy_for_hosted() {
    let fixture = Fixture::new();
    let values = kwargs();
    let hosted = call_with(hosted_params(), Some(50), None, None, &values);
    let mut state = logging();
    fixture
        .service()
        .list_skills(listing(&hosted, &mut state))
        .await
        .expect("the hosted list succeeds");
    assert_eq!(state.custom_llm_provider.as_deref(), Some("litellm_proxy"));
    assert_eq!(
        state.optional_params,
        map(&[
            ("limit", Json::Number(Number::from(50_i64))),
            ("offset", Json::Number(Number::from(0_i64))),
        ])
    );
}

#[tokio::test]
async fn list_skill_logging_keeps_hosted_kwargs_out_of_litellm_params() {
    let fixture = Fixture::new();
    let principal = object(&[("user_id", text("user-1"))]);
    let mut values = kwargs();
    values.insert("metadata".to_owned(), metadata_with(principal));
    let hosted = call_with(hosted_params(), None, None, None, &values);
    let mut state = logging();
    fixture
        .service()
        .list_skills(listing(&hosted, &mut state))
        .await
        .expect("the hosted list succeeds");
    assert!(state.litellm_params.get("metadata").is_none());
    assert!(state.litellm_params.get("litellm_metadata").is_none());
    assert_eq!(
        state.optional_params.get("limit"),
        Some(&Json::Number(Number::from(20_i64)))
    );
}

#[tokio::test]
async fn list_skill_logging_records_the_resolved_hosted_default() {
    let fixture = Fixture::new();
    let values = kwargs();
    let hosted = call_with(hosted_params(), None, None, None, &values);
    let mut state = logging();
    fixture
        .service()
        .list_skills(listing(&hosted, &mut state))
        .await
        .expect("the hosted list succeeds");
    assert_eq!(
        state.optional_params.get("limit"),
        Some(&Json::Number(Number::from(20_i64)))
    );
}
