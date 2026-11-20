#![cfg(test)]

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::containers::container_file_http::request::{ParamValue, SslVerify};
use betterlite_openai::containers::container_file_http::transport::ContainerMethod;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Number;

use super::caller_defaults;
use super::prepare::{Prepared, prepare_call};
use super::test_support::{
    API_KEY, CALL_ID, CONTAINER_ID, FILE_LIST_BODY, FILES_BASE, FakeClient, Fixture, Lookup,
    REQUEST_TIMEOUT, RecordingHook, object,
};
use crate::api::response_id_codecs::container::build_container_id;

fn file_url(container_id: &str) -> String {
    format!("{FILES_BASE}/{container_id}/files")
}

fn prepared_timeout(fixture: &Fixture) -> f64 {
    let request = fixture.request();
    let prepared = prepare_call(
        &fixture.lookup,
        &fixture.deps(),
        &request,
        caller_defaults(&request),
    )
    .expect("the call prepares");
    let Prepared::Call(ready) = prepared else {
        panic!("the fixture carries no mock response");
    };
    ready.timeout_seconds
}

#[test]
fn a_file_list_call_sends_one_get_with_the_transformed_url_and_query() {
    let mut fixture = Fixture::new();
    fixture.after = Some("cfile_0".to_owned());
    fixture.limit = Some(5);
    fixture.order = Some("asc".to_owned());

    let response = fixture.call().expect("the file list decodes");

    assert_eq!(response.first_id.as_deref(), Some("cfile_1"));
    assert!(!response.has_more);
    assert_eq!(response.data.len(), 1);
    assert_eq!(response.data[0].id, "cfile_1");
    let requests = fixture.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, ContainerMethod::Get);
    assert_eq!(requests[0].url, file_url(CONTAINER_ID));
    assert!(requests[0].file.is_none());
    assert_eq!(
        requests[0].query,
        vec![
            ("after".to_owned(), "cfile_0".to_owned()),
            ("limit".to_owned(), "5".to_owned()),
            ("order".to_owned(), "asc".to_owned()),
        ]
    );
}

#[test]
fn an_empty_transform_query_sends_no_query_pairs() {
    let fixture = Fixture::new();

    fixture.call().expect("the file list decodes");

    let requests = fixture.requests();
    assert!(requests[0].query.is_empty());
    assert_eq!(requests[0].url, file_url(CONTAINER_ID));
}

#[test]
fn the_declared_timeout_is_kept_when_the_caller_gives_one() {
    let mut fixture = Fixture::new();
    fixture.timeout = Some(5.0);

    let timeout = prepared_timeout(&fixture);

    assert_eq!(timeout.partial_cmp(&5.0_f64), Some(Ordering::Equal));
}

#[test]
fn an_absent_timeout_uses_the_declared_six_hundred_seconds() {
    let fixture = Fixture::new();

    let timeout = prepared_timeout(&fixture);

    assert_eq!(timeout.partial_cmp(&600.0_f64), Some(Ordering::Equal));
}

#[test]
fn a_zero_timeout_falls_back_to_the_request_timeout() {
    let mut fixture = Fixture::new();
    fixture.timeout = Some(0.0);

    let timeout = prepared_timeout(&fixture);

    assert_eq!(timeout.partial_cmp(&REQUEST_TIMEOUT), Some(Ordering::Equal));
}

#[test]
fn the_timeout_never_reaches_the_request() {
    let mut patient = Fixture::new();
    patient.timeout = Some(600.0);
    let mut impatient = Fixture::new();
    impatient.timeout = Some(1.0);

    patient.call().expect("the file list decodes");
    impatient.call().expect("the file list decodes");

    assert_eq!(patient.requests(), impatient.requests());
}

#[test]
fn a_mock_string_short_circuits_the_provider_lookup_and_the_send() {
    let mut fixture = Fixture::new();
    fixture.kwargs.insert(
        "mock_response".to_owned(),
        Json::Text(FILE_LIST_BODY.to_owned()),
    );
    fixture.provider = Some("acme".to_owned());

    let response = fixture.call().expect("the mock answers without a provider");

    assert_eq!(response.first_id.as_deref(), Some("cfile_1"));
    assert!(fixture.lookup.providers().is_empty());
    assert!(fixture.requests().is_empty());
    assert!(fixture.hook.calls().is_empty());
}

#[test]
fn a_mock_object_decodes_like_a_mock_string() {
    let mut fixture = Fixture::new();
    let mock = serde_json::from_str::<Json>(FILE_LIST_BODY).expect("the fixture is JSON");
    fixture.kwargs.insert("mock_response".to_owned(), mock);

    let response = fixture.call().expect("the mock object answers");

    assert_eq!(response.first_id.as_deref(), Some("cfile_1"));
    assert!(fixture.requests().is_empty());
}

#[test]
fn an_unparseable_mock_response_is_a_value_error() {
    let mut fixture = Fixture::new();
    fixture.kwargs.insert(
        "mock_response".to_owned(),
        Json::Text("not json".to_owned()),
    );

    let failure = fixture.call().expect_err("the mock does not parse");

    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected an api connection, got {:?}", failure.class);
    };
    assert_eq!(context.provider.as_deref(), Some("openai"));
    assert_eq!(context.model.as_deref(), Some(""));
    assert!(fixture.requests().is_empty());
}

#[test]
fn a_mock_response_of_the_wrong_shape_is_a_value_error() {
    let mut fixture = Fixture::new();
    fixture.kwargs.insert(
        "mock_response".to_owned(),
        Json::Text(r#"{"object":"list"}"#.to_owned()),
    );

    let failure = fixture.call().expect_err("the mock misses its data");

    assert!(matches!(&failure.class, ErrorClass::ApiConnection(_)));
    assert!(fixture.requests().is_empty());
}

#[test]
fn a_managed_container_id_resolves_its_provider_and_sends_the_original_id() {
    let mut fixture = Fixture::new();
    fixture.container_id = build_container_id(Some("azure"), None, CONTAINER_ID);

    let response = fixture.call().expect("the file list decodes");

    assert_eq!(fixture.lookup.providers(), vec![ProviderId::Azure]);
    let requests = fixture.requests();
    assert_eq!(requests[0].url, file_url(CONTAINER_ID));
    assert_eq!(response.data[0].container_id, CONTAINER_ID);
    assert_eq!(response.first_id.as_deref(), Some("cfile_1"));
}

#[test]
fn an_explicit_provider_is_the_one_looked_up() {
    let mut fixture = Fixture::new();
    fixture.provider = Some("azure_text".to_owned());

    fixture.call().expect("the file list decodes");

    assert_eq!(fixture.lookup.providers(), vec![ProviderId::Azure]);
}

#[test]
fn an_explicit_provider_wins_over_the_managed_one() {
    let mut fixture = Fixture::new();
    fixture.container_id = build_container_id(Some("openai"), None, CONTAINER_ID);
    fixture.provider = Some("azure".to_owned());

    fixture.call().expect("the file list decodes");

    assert_eq!(fixture.lookup.providers(), vec![ProviderId::Azure]);
}

#[test]
fn a_managed_provider_wins_over_the_openai_default() {
    let mut fixture = Fixture::new();
    fixture.container_id = build_container_id(Some("azure"), None, CONTAINER_ID);
    fixture.provider = Some("openai".to_owned());

    fixture.call().expect("the file list decodes");

    assert_eq!(fixture.lookup.providers(), vec![ProviderId::Azure]);
}

#[test]
fn a_provider_without_a_container_config_is_a_clear_error() {
    let mut fixture = Fixture::new();
    fixture.provider = Some("acme".to_owned());

    let failure = fixture
        .call()
        .expect_err("no container config answers acme");

    assert_eq!(
        failure.error.to_string(),
        "Container provider config not found for provider: acme"
    );
    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected an api connection, got {:?}", failure.class);
    };
    assert_eq!(context.provider.as_deref(), Some("acme"));
    assert!(fixture.lookup.providers().is_empty());
    assert!(fixture.requests().is_empty());
}

#[test]
fn a_provider_whose_config_is_missing_is_the_same_error() {
    let mut fixture = Fixture::new();
    fixture.lookup = Lookup::empty();

    let failure = fixture.call().expect_err("the registry answers no config");

    assert_eq!(
        failure.error.to_string(),
        "Container provider config not found for provider: openai"
    );
    assert_eq!(fixture.lookup.providers(), vec![ProviderId::Openai]);
    assert!(fixture.requests().is_empty());
}

#[test]
fn the_logging_state_carries_the_call_type_and_pagination_params() {
    let mut fixture = Fixture::new();
    fixture.after = Some("cfile_0".to_owned());
    fixture.limit = Some(5);
    fixture.order = Some("asc".to_owned());

    fixture.call().expect("the file list decodes");

    let state = fixture
        .logging
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(state.call_type, "list_container_files");
    assert_eq!(state.model, "");
    assert_eq!(state.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        state.optional_params.get("container_id"),
        Some(&Json::Text(CONTAINER_ID.to_owned()))
    );
    assert_eq!(
        state.optional_params.get("after"),
        Some(&Json::Text("cfile_0".to_owned()))
    );
    assert_eq!(
        state.optional_params.get("limit"),
        Some(&Json::Number(Number::from(5_i64)))
    );
    assert_eq!(
        state.optional_params.get("order"),
        Some(&Json::Text("asc".to_owned()))
    );
    assert_eq!(
        state.litellm_params.get("litellm_call_id"),
        Some(&Json::Text(CALL_ID.to_owned()))
    );
}

#[test]
fn the_logged_optional_params_keep_a_null_for_what_the_caller_left_out() {
    let fixture = Fixture::new();

    fixture.call().expect("the file list decodes");

    let state = fixture
        .logging
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(state.optional_params.get("after"), Some(&Json::Null));
    assert_eq!(state.optional_params.get("limit"), Some(&Json::Null));
    assert_eq!(state.optional_params.get("order"), Some(&Json::Null));
}

#[test]
fn the_logging_object_and_async_flag_never_reach_the_params() {
    let mut fixture = Fixture::new();
    fixture.kwargs.insert(
        "litellm_logging_obj".to_owned(),
        Json::Text("object".to_owned()),
    );
    fixture
        .kwargs
        .insert("async_call".to_owned(), Json::Bool(true));

    fixture.call().expect("the file list decodes");

    let state = fixture
        .logging
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert!(state.litellm_params.get("litellm_logging_obj").is_none());
    assert!(state.litellm_params.get("async_call").is_none());
    let transforms = fixture.config().transforms();
    assert!(transforms[0].params.get("litellm_logging_obj").is_none());
    assert!(transforms[0].params.get("async_call").is_none());
}

#[test]
fn pre_call_runs_before_the_send_with_the_final_url_headers_and_query() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let mut fixture = Fixture::new();
    fixture.hook = RecordingHook::passing().with_order(Arc::clone(&order));
    fixture.source.client =
        FakeClient::answering(200, FILE_LIST_BODY).with_order(Arc::clone(&order));
    fixture.limit = Some(5);

    fixture.call().expect("the file list decodes");

    let calls = fixture.hook.calls();
    assert_eq!(calls.len(), 1);
    let seen = &calls[0];
    assert_eq!(seen.call_type, "list_container_files");
    assert_eq!(seen.provider.as_deref(), Some("openai"));
    assert_eq!(seen.input, Json::Text(String::new()));
    assert_eq!(
        seen.args.api_base.as_deref(),
        Some(file_url(CONTAINER_ID).as_str())
    );
    let headers = seen
        .args
        .headers
        .as_ref()
        .expect("pre_call carries headers");
    assert_eq!(
        headers.get("Authorization"),
        Some(&Json::Text("Bearer None".to_owned()))
    );
    let Some(Json::Object(params)) = seen.args.passthrough.get("params") else {
        panic!("pre_call carries the params");
    };
    assert_eq!(params.get("limit"), Some(&Json::Text("5".to_owned())));
    let recorded = order.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(*recorded, vec!["pre_call".to_owned(), "send".to_owned()]);
}

#[test]
fn extra_headers_are_laid_over_the_provider_headers() {
    let mut fixture = Fixture::new();
    fixture.extra_headers = Some(BTreeMap::from([
        ("Authorization".to_owned(), "Bearer custom".to_owned()),
        ("X-Custom".to_owned(), "1".to_owned()),
    ]));

    fixture.call().expect("the file list decodes");

    let environment = fixture.config().environment();
    assert_eq!(environment.len(), 1);
    assert_eq!(
        environment[0]
            .headers
            .get("Authorization")
            .map(String::as_str),
        Some("Bearer custom")
    );
    let requests = fixture.requests();
    assert_eq!(
        requests[0].headers.get("Authorization").map(String::as_str),
        Some("Bearer custom")
    );
    assert_eq!(
        requests[0].headers.get("X-Custom").map(String::as_str),
        Some("1")
    );
}

#[test]
fn an_api_key_reaches_validate_environment_and_authorizes_the_request() {
    let mut fixture = Fixture::new();
    fixture.api_key = Some(SecretString::from(API_KEY));

    fixture.call().expect("the file list decodes");

    let environment = fixture.config().environment();
    assert_eq!(environment[0].api_key.as_deref(), Some(API_KEY));
    let requests = fixture.requests();
    assert_eq!(
        requests[0].headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-test")
    );
}

#[test]
fn a_ssl_verify_kwarg_reaches_the_transform_and_the_client_builder() {
    let mut fixture = Fixture::new();
    fixture
        .kwargs
        .insert("ssl_verify".to_owned(), Json::Bool(true));

    fixture.call().expect("the file list decodes");

    assert_eq!(
        fixture.source.blocking_ssl(),
        vec![Some(SslVerify::Enabled(true))]
    );
    let transforms = fixture.config().transforms();
    assert_eq!(
        transforms[0]
            .params
            .get("ssl_verify")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
}

#[test]
fn api_base_and_api_version_reach_the_provider_config() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some("https://proxy.example/v1".to_owned());
    fixture.api_version = Some("2024-06-01".to_owned());

    fixture.call().expect("the file list decodes");

    let bases = fixture.config().bases();
    assert_eq!(bases.len(), 1);
    assert_eq!(
        bases[0].api_base.as_deref(),
        Some("https://proxy.example/v1")
    );
    assert_eq!(
        bases[0]
            .params
            .get("api_version")
            .and_then(|value| value.as_str()),
        Some("2024-06-01")
    );
    let transforms = fixture.config().transforms();
    assert_eq!(
        transforms[0].api_base,
        "https://proxy.example/v1/containers"
    );
    assert_eq!(
        transforms[0]
            .params
            .get("api_base")
            .and_then(|value| value.as_str()),
        Some("https://proxy.example/v1")
    );
    let requests = fixture.requests();
    assert_eq!(
        requests[0].url,
        "https://proxy.example/v1/containers/cntr_original/files"
    );
}

#[test]
fn pagination_and_extra_query_are_rendered_as_query_pairs() {
    let mut fixture = Fixture::new();
    fixture.after = Some("cfile_0".to_owned());
    fixture.limit = Some(5);
    fixture.order = Some("asc".to_owned());
    fixture.extra_query = Some(IndexMap::from([
        ("api-version".to_owned(), ParamValue::Text("v1".to_owned())),
        ("z".to_owned(), ParamValue::Integer(2)),
    ]));

    fixture.call().expect("the file list decodes");

    let requests = fixture.requests();
    assert_eq!(
        requests[0].query,
        vec![
            ("after".to_owned(), "cfile_0".to_owned()),
            ("api-version".to_owned(), "v1".to_owned()),
            ("limit".to_owned(), "5".to_owned()),
            ("order".to_owned(), "asc".to_owned()),
            ("z".to_owned(), "2".to_owned()),
        ]
    );
    let transforms = fixture.config().transforms();
    assert_eq!(
        transforms[0].extra_query,
        Some(object(r#"{"api-version":"v1","z":2}"#))
    );
}

#[test]
fn a_provider_error_status_keeps_its_status_through_the_operation() {
    let fixture = Fixture::answering(
        404,
        r#"{"error":{"message":"No such container: cntr_original"}}"#,
    );

    let failure = fixture.call().expect_err("the provider answered 404");

    let ErrorClass::NotFound(details) = &failure.class else {
        panic!("expected a not found, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 404);
    assert_eq!(details.provider.as_deref(), Some("openai"));
    assert_eq!(details.model.as_deref(), Some(""));
    assert!(details.message.contains("No such container: cntr_original"));
}

#[test]
fn a_transport_failure_is_an_internal_server() {
    let mut fixture = Fixture::new();
    fixture.source.client = FakeClient::failing();

    let failure = fixture.call().expect_err("the client is closed");

    let ErrorClass::InternalServer(context) = &failure.class else {
        panic!("expected an internal server, got {:?}", failure.class);
    };
    assert!(context.message.contains("the client is shutting down"));
    assert_eq!(context.provider.as_deref(), Some("openai"));
    assert_eq!(context.model.as_deref(), Some(""));
}

#[tokio::test]
async fn the_async_operation_sends_through_the_async_client() {
    let fixture = Fixture::new();

    let response = fixture.async_call().await.expect("the file list decodes");

    assert_eq!(response.first_id.as_deref(), Some("cfile_1"));
    assert_eq!(fixture.requests().len(), 1);
    assert!(fixture.source.blocking_ssl().is_empty());
    assert_eq!(fixture.source.async_ssl().len(), 1);
}

#[tokio::test]
async fn a_mock_response_short_circuits_the_async_send() {
    let mut fixture = Fixture::new();
    fixture.kwargs.insert(
        "mock_response".to_owned(),
        Json::Text(FILE_LIST_BODY.to_owned()),
    );

    let response = fixture.async_call().await.expect("the mock answers");

    assert_eq!(response.first_id.as_deref(), Some("cfile_1"));
    assert!(fixture.source.async_ssl().is_empty());
    assert!(fixture.requests().is_empty());
}

#[tokio::test]
async fn a_transport_failure_through_the_async_operation_is_an_internal_server() {
    let mut fixture = Fixture::new();
    fixture.source.client = FakeClient::failing();

    let failure = fixture
        .async_call()
        .await
        .expect_err("the client is closed");

    assert!(matches!(&failure.class, ErrorClass::InternalServer(_)));
}
