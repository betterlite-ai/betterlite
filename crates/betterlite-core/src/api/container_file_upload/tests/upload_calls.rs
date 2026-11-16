#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_error::error::ErrorClass;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::containers::container_file_http::request::ParamValue;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;

use super::ENDPOINT_NAME;
use super::test_support::{
    API_BASE, API_KEY, API_VERSION, CALL_ID, CONTAINER_ID, Fixture, REQUEST_TIMEOUT, StubExecutor,
    file, object,
};
use super::upload_container_file;
use crate::api::response_id_codecs::container::build_container_id;
use crate::containers::container_request_parameters::ContainerProviderConfig;

#[test]
fn upload_uses_the_openai_provider_and_the_six_hundred_second_default() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    let reply =
        upload_container_file(&executor, fixture.deps(), &request).expect("the upload succeeds");

    assert_eq!(reply, object("file_1"));
    let seen = executor.seen().expect("the executor ran");
    assert_eq!(seen.endpoint_name, ENDPOINT_NAME);
    assert_eq!(seen.endpoint_name, "upload_container_file");
    assert_eq!(seen.provider_config, ContainerProviderConfig::OpenAi);
    assert_eq!(seen.timeout_seconds, 600.0);
    assert_eq!(seen.file, file());
}

#[test]
fn upload_passes_named_credentials_in_litellm_params() {
    let mut fixture = Fixture::new();
    fixture.api_key = Some(SecretString::from(API_KEY));
    fixture.api_base = Some(API_BASE.to_owned());
    fixture.api_version = Some(API_VERSION.to_owned());
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    upload_container_file(&executor, fixture.deps(), &request).expect("the upload succeeds");

    let seen = executor.seen().expect("the executor ran");
    assert_eq!(
        seen.litellm_params.get("api_key"),
        Some(&JsonValue::Text(API_KEY.to_owned()))
    );
    assert_eq!(
        seen.litellm_params.get("api_base"),
        Some(&JsonValue::Text(API_BASE.to_owned()))
    );
    assert_eq!(
        seen.litellm_params.get("api_version"),
        Some(&JsonValue::Text(API_VERSION.to_owned()))
    );
    assert_eq!(
        seen.litellm_params.get("litellm_call_id"),
        Some(&JsonValue::Text(CALL_ID.to_owned()))
    );
}

#[test]
fn a_named_credential_replaces_the_same_kwarg() {
    let mut fixture = Fixture::new();
    fixture.api_key = Some(SecretString::from(API_KEY));
    fixture
        .kwargs
        .insert("api_key".to_owned(), Json::Text("from-kwargs".to_owned()));
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    upload_container_file(&executor, fixture.deps(), &request).expect("the upload succeeds");

    let seen = executor.seen().expect("the executor ran");
    assert_eq!(
        seen.litellm_params.get("api_key"),
        Some(&JsonValue::Text(API_KEY.to_owned()))
    );
}

#[test]
fn upload_passes_scalar_kwargs_in_litellm_params() {
    let mut fixture = Fixture::new();
    fixture
        .kwargs
        .insert("ssl_verify".to_owned(), Json::Bool(false));
    fixture
        .kwargs
        .insert("organization".to_owned(), Json::Text("org-1".to_owned()));
    fixture.kwargs.insert(
        "top_p".to_owned(),
        Json::Number(Number::from_f64(0.5).expect("the fixture float is finite")),
    );
    fixture.kwargs.insert("stop".to_owned(), Json::Null);
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    upload_container_file(&executor, fixture.deps(), &request).expect("the upload succeeds");

    let seen = executor.seen().expect("the executor ran");
    assert_eq!(
        seen.litellm_params.get("ssl_verify"),
        Some(&JsonValue::Bool(false))
    );
    assert_eq!(
        seen.litellm_params.get("organization"),
        Some(&JsonValue::Text("org-1".to_owned()))
    );
    assert_eq!(
        seen.litellm_params.get("top_p"),
        Some(&JsonValue::Number(
            Number::from_f64(0.5).expect("the fixture float is finite")
        ))
    );
    assert_eq!(seen.litellm_params.get("stop"), Some(&JsonValue::Null));
}

#[test]
fn upload_passes_container_kwargs_in_litellm_params() {
    let mut fixture = Fixture::new();
    fixture.kwargs.insert(
        "tags".to_owned(),
        Json::List(vec![Json::Text("one".to_owned()), Json::Bool(true)]),
    );
    fixture.kwargs.insert(
        "extra".to_owned(),
        Json::Object(IndexMap::from([(
            "nested".to_owned(),
            Json::Number(Number::from(7_i64)),
        )])),
    );
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    upload_container_file(&executor, fixture.deps(), &request).expect("the upload succeeds");

    let seen = executor.seen().expect("the executor ran");
    assert_eq!(
        seen.litellm_params.get("tags"),
        Some(&JsonValue::Array(vec![
            JsonValue::Text("one".to_owned()),
            JsonValue::Bool(true),
        ]))
    );
    assert_eq!(
        seen.litellm_params.get("extra"),
        Some(&JsonValue::Object(IndexMap::from([(
            "nested".to_owned(),
            JsonValue::Number(Number::from(7_i64)),
        )])))
    );
}

#[test]
fn upload_forwards_extra_headers_and_extra_query() {
    let mut fixture = Fixture::new();
    fixture.extra_headers = Some(BTreeMap::from([("x-extra".to_owned(), "1".to_owned())]));
    fixture.extra_query = Some(IndexMap::from([(
        "after".to_owned(),
        ParamValue::Text("file_0".to_owned()),
    )]));
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    upload_container_file(&executor, fixture.deps(), &request).expect("the upload succeeds");

    let seen = executor.seen().expect("the executor ran");
    assert_eq!(seen.extra_headers, fixture.extra_headers);
    assert_eq!(seen.extra_query, fixture.extra_query);
}

#[test]
fn upload_sends_the_decoded_container_id_and_the_original_file() {
    let mut fixture = Fixture::new();
    let managed = build_container_id(Some("openai"), None, CONTAINER_ID);
    fixture.container_id = managed.clone();
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    upload_container_file(&executor, fixture.deps(), &request).expect("the upload succeeds");

    let seen = executor.seen().expect("the executor ran");
    assert_eq!(seen.container_id, CONTAINER_ID);
    assert_eq!(seen.file, file());
    assert_eq!(
        seen.optional_params.get("container_id"),
        Some(&Json::Text(managed))
    );
}

#[test]
fn upload_takes_the_provider_from_the_managed_id() {
    let mut fixture = Fixture::new();
    fixture.container_id = build_container_id(Some("azure"), Some("gpt-4o"), CONTAINER_ID);
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    upload_container_file(&executor, fixture.deps(), &request).expect("the upload succeeds");

    let seen = executor.seen().expect("the executor ran");
    assert_eq!(seen.provider_config, ContainerProviderConfig::Azure);
    assert_eq!(seen.container_id, CONTAINER_ID);
    assert_eq!(
        seen.litellm_params.get("model_id"),
        Some(&JsonValue::Text("gpt-4o".to_owned()))
    );
}

#[test]
fn upload_keeps_an_explicit_non_openai_provider() {
    let mut fixture = Fixture::new();
    fixture.container_id = build_container_id(Some("openai"), None, CONTAINER_ID);
    fixture.provider = Some("azure".to_owned());
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    upload_container_file(&executor, fixture.deps(), &request).expect("the upload succeeds");

    let seen = executor.seen().expect("the executor ran");
    assert_eq!(seen.provider_config, ContainerProviderConfig::Azure);
    assert_eq!(seen.container_id, CONTAINER_ID);
}

#[rstest]
#[case::omitted(None, 600.0_f64)]
#[case::explicit_zero(Some(0.0_f64), REQUEST_TIMEOUT)]
#[case::negative_zero(Some(-0.0_f64), REQUEST_TIMEOUT)]
#[case::seconds(Some(2.5_f64), 2.5_f64)]
#[case::negative(Some(-1.0_f64), -1.0_f64)]
fn upload_resolves_the_timeout(#[case] timeout: Option<f64>, #[case] expected: f64) {
    let mut fixture = Fixture::new();
    fixture.timeout = timeout;
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    upload_container_file(&executor, fixture.deps(), &request).expect("the upload succeeds");

    assert_eq!(
        executor.seen().expect("the executor ran").timeout_seconds,
        expected
    );
}

#[test]
fn upload_without_a_container_config_is_an_api_connection_failure() {
    let mut fixture = Fixture::new();
    fixture.provider = Some("bogus".to_owned());
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    let error = upload_container_file(&executor, fixture.deps(), &request)
        .expect_err("bogus has no container config");

    let ErrorClass::ApiConnection(context) = &error.class else {
        panic!("expected an api connection, got {:?}", error.class);
    };
    assert_eq!(context.provider.as_deref(), Some("bogus"));
    assert_eq!(context.model.as_deref(), Some(""));
    assert!(!executor.was_called());
}

#[test]
fn upload_for_a_known_provider_without_a_container_config_fails_the_same_way() {
    let mut fixture = Fixture::new();
    fixture.provider = Some("anthropic".to_owned());
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    let error = upload_container_file(&executor, fixture.deps(), &request)
        .expect_err("anthropic has no container config");

    let ErrorClass::ApiConnection(context) = &error.class else {
        panic!("expected an api connection, got {:?}", error.class);
    };
    assert_eq!(context.provider.as_deref(), Some("anthropic"));
    assert!(!executor.was_called());
}

#[test]
fn upload_rejects_a_wrong_typed_model_id_from_the_managed_id() {
    let mut fixture = Fixture::new();
    fixture.container_id = build_container_id(Some("openai"), Some("gpt-4o"), CONTAINER_ID);
    fixture
        .kwargs
        .insert("model_id".to_owned(), Json::Number(Number::from(5_i64)));
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    let error = upload_container_file(&executor, fixture.deps(), &request)
        .expect_err("a number is not a model id");

    let ErrorClass::BadRequest(details) = &error.class else {
        panic!("expected a bad request, got {:?}", error.class);
    };
    assert_eq!(
        details.body_param.as_deref(),
        Some("litellm_params.model_id")
    );
    assert_eq!(details.status.0, 400);
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some("openai"));
    assert!(!executor.was_called());
}

#[test]
fn upload_never_forwards_extra_body() {
    let mut fixture = Fixture::new();
    fixture.extra_body = Some(IndexMap::from([(
        "metadata".to_owned(),
        JsonValue::Text("body-only".to_owned()),
    )]));
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    upload_container_file(&executor, fixture.deps(), &request).expect("the upload succeeds");

    let seen = executor.seen().expect("the executor ran");
    assert_eq!(seen.litellm_params.len(), 1);
    assert!(!seen.litellm_params.contains_key("extra_body"));
    assert!(!seen.litellm_params.contains_key("metadata"));
}

#[test]
fn upload_drops_the_async_call_flag_from_the_handler_params() {
    let mut fixture = Fixture::new();
    fixture
        .kwargs
        .insert("async_call".to_owned(), Json::Bool(true));
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    upload_container_file(&executor, fixture.deps(), &request).expect("the upload succeeds");

    let seen = executor.seen().expect("the executor ran");
    assert_eq!(seen.litellm_params.len(), 1);
    assert!(!seen.litellm_params.contains_key("async_call"));
    assert_eq!(
        seen.litellm_params.get("litellm_call_id"),
        Some(&JsonValue::Text(CALL_ID.to_owned()))
    );
}
