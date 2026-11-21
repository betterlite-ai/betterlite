#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::containers::container_file_http::request::ParamValue;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::test_support::{
    CONTAINER_ID, Fixture, StubDecoder, StubHandler, decoding_to, deleted_result,
};

#[test]
fn handler_args_the_headers_query_and_timeout_reach_the_handler() {
    let mut fixture = Fixture::new();
    fixture.timeout = Some(Duration::from_secs(30));
    fixture.extra_headers = Some(BTreeMap::from([("x-extra".to_owned(), "1".to_owned())]));
    fixture.extra_query = Some(IndexMap::from([(
        "api-version".to_owned(),
        ParamValue::Text("2".to_owned()),
    )]));

    fixture.retrieve().expect("the call runs");

    let calls = fixture.calls();
    let call = calls.first().expect("one call");
    assert_eq!(call.timeout, Some(Duration::from_secs(30)));
    assert_eq!(call.headers, fixture.extra_headers);
    assert_eq!(call.query, fixture.extra_query);
}

#[test]
fn handler_args_a_missing_timeout_is_the_declared_six_hundred_seconds() {
    let fixture = Fixture::new();

    fixture.retrieve().expect("the call runs");

    assert_eq!(
        fixture.calls().first().expect("one call").timeout,
        Some(Duration::from_secs(600))
    );
}

#[test]
fn handler_args_a_zero_timeout_is_the_configured_request_timeout() {
    let mut fixture = Fixture::new();
    fixture.timeout = Some(Duration::ZERO);

    fixture.retrieve().expect("the call runs");

    assert_eq!(
        fixture.calls().first().expect("one call").timeout,
        Some(Duration::from_mins(100))
    );
}

#[test]
fn handler_args_the_params_carry_the_credential_and_the_decoded_model_id() {
    let mut fixture = Fixture::new();
    fixture.api_key = Some(SecretString::from("sk-test"));
    fixture.decoder =
        StubDecoder::answering(decoding_to(CONTAINER_ID, "openai", Some("deployment")));

    fixture.retrieve().expect("the call runs");

    let calls = fixture.calls();
    let params = &calls.first().expect("one call").params;
    assert_eq!(
        params.get("api_key").and_then(|key| key.as_str()),
        Some("sk-test")
    );
    assert_eq!(
        params.get("model_id").and_then(|value| value.as_str()),
        Some("deployment")
    );
}

// GenericLiteLLMParams(**kwargs) keeps the call id, the metadata and a null mock response in the params
#[test]
fn handler_args_the_callers_kwargs_travel_in_the_params() {
    let mut fixture = Fixture::new();
    fixture.litellm_metadata = Some(IndexMap::from([(
        "model_info".to_owned(),
        Json::Object(IndexMap::new()),
    )]));
    fixture.mock_response = Some(Json::Null);

    fixture.retrieve().expect("the call runs");

    let calls = fixture.calls();
    let params = &calls.first().expect("one call").params;
    assert_eq!(
        params
            .get("litellm_call_id")
            .and_then(|value| value.as_str()),
        Some("call-1")
    );
    assert!(params["litellm_metadata"].is_object());
    assert!(params["mock_response"].is_null());
    assert!(params.get("extra_body").is_none());
}

// GenericLiteLLMParams keeps every extra kwarg, and B3365 reads ssl_verify from the params to pick its TLS client
#[test]
fn handler_args_an_extra_ssl_verify_reaches_the_handler_params() {
    let mut fixture = Fixture::new();
    fixture.extras = Some(IndexMap::from([(
        "ssl_verify".to_owned(),
        Json::Bool(false),
    )]));

    fixture.retrieve().expect("the call runs");

    let calls = fixture.calls();
    let params = &calls.first().expect("one call").params;
    assert_eq!(params["ssl_verify"].as_bool(), Some(false));
}

#[test]
fn handler_args_a_named_argument_wins_over_an_extra_with_the_same_name() {
    let mut fixture = Fixture::new();
    fixture.api_key = Some(SecretString::from("sk-named"));
    fixture.extras = Some(IndexMap::from([(
        "api_key".to_owned(),
        Json::Text("sk-extra".to_owned()),
    )]));

    fixture.retrieve().expect("the call runs");

    let calls = fixture.calls();
    let params = &calls.first().expect("one call").params;
    assert_eq!(
        params.get("api_key").and_then(|key| key.as_str()),
        Some("sk-named")
    );
}

#[test]
fn handler_args_extra_body_is_accepted_and_never_reaches_the_handler() {
    let mut fixture = Fixture::new();
    fixture.extra_body = Some(IndexMap::from([(
        "target_model_names".to_owned(),
        JsonValue::Array(vec![JsonValue::Text("gpt-4o".to_owned())]),
    )]));

    let object = fixture.retrieve().expect("the call runs");

    assert_eq!(
        fixture
            .calls()
            .first()
            .expect("one call")
            .params
            .get("target_model_names"),
        None
    );
    assert_eq!(object.id, CONTAINER_ID);
}

#[test]
fn handler_args_the_delete_call_carries_the_same_arguments() {
    let mut fixture = Fixture::new();
    fixture.handler = StubHandler::deleting(deleted_result());
    fixture.timeout = Some(Duration::from_secs(5));
    fixture.extra_headers = Some(BTreeMap::from([("x-extra".to_owned(), "1".to_owned())]));

    fixture.delete().expect("the call runs");

    let calls = fixture.calls();
    let call = calls.first().expect("one call");
    assert_eq!(call.container_id, CONTAINER_ID);
    assert_eq!(call.timeout, Some(Duration::from_secs(5)));
    assert_eq!(call.headers, fixture.extra_headers);
    assert_eq!(
        call.litellm_params,
        IndexMap::from([(
            "litellm_call_id".to_owned(),
            Json::Text("call-1".to_owned())
        )])
    );
}
