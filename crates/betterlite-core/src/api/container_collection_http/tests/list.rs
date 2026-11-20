#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_openai::api::container_contracts::config::ContainerListQuery;
use betterlite_openai::containers::container_file_http::request::SslVerify;
use betterlite_openai::containers::container_file_http::transport::ContainerMethod;
use betterlite_openai::openai_base_config::Object;
use serde_json::json;

use super::tests::{
    API_VERSION, BASE, BlockingFake, FakeClient, Fixture, LIST_BODY, deadline_failure,
    provider_error, url_failure,
};
use super::{
    CollectionRequest, ContainerCollectionError, acontainer_list_handler, container_list_handler,
};

fn all_page() -> ContainerListQuery<'static> {
    ContainerListQuery {
        after: Some("cntr_a"),
        limit: Some(2),
        order: Some("asc"),
        extra_query: None,
    }
}

fn empty_page() -> ContainerListQuery<'static> {
    ContainerListQuery {
        after: None,
        limit: None,
        order: None,
        extra_query: None,
    }
}

fn api_key(value: &str) -> Object {
    let mut params = Object::new();
    params.insert(String::from("api_key"), json!(value));
    params
}

fn rendered(request: &CollectionRequest) -> BTreeMap<String, String> {
    request.query.iter().cloned().collect()
}

fn api_base_with_version() -> String {
    format!("{BASE}?api-version={API_VERSION}")
}

#[test]
fn list_sends_a_get_with_the_rendered_query_and_no_timeout() {
    let fixture = Fixture::new(200, LIST_BODY);
    let call = fixture.list_call(all_page(), Some(Duration::from_secs(5)));
    let page = container_list_handler(&call, &fixture.source, None, &fixture.hook).unwrap();
    let sent = fixture.sent();
    assert_eq!(sent.len(), 1);
    let request = &sent[0];
    assert_eq!(request.method, ContainerMethod::Get);
    assert_eq!(request.url, api_base_with_version());
    assert_eq!(
        rendered(request),
        BTreeMap::from([
            (String::from("after"), String::from("cntr_a")),
            (String::from("limit"), String::from("2")),
            (String::from("order"), String::from("asc")),
        ])
    );
    assert!(request.body.is_none());
    assert_eq!(request.timeout, None, "the GET keeps the client's deadline");
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].id, "cntr_a");
    assert_eq!(page.first_id.as_deref(), Some("cntr_a"));
    assert!(!page.has_more);
}

#[test]
fn list_keeps_the_url_query_when_the_params_are_empty() {
    let fixture = Fixture::new(200, LIST_BODY);
    let call = fixture.list_call(empty_page(), None);
    container_list_handler(&call, &fixture.source, None, &fixture.hook).unwrap();
    let sent = fixture.sent();
    assert!(sent[0].query.is_empty(), "nothing is sent as params");
    assert_eq!(
        sent[0].url,
        api_base_with_version(),
        "the api-version already in the base URL stays"
    );
}

#[test]
fn list_renders_every_extra_query_value_as_httpx_would() {
    let mut extra = Object::new();
    extra.insert(String::from("page"), json!(5));
    extra.insert(String::from("flag"), json!(true));
    extra.insert(String::from("none"), json!(null));
    extra.insert(String::from("name"), json!("x"));
    let mut fixture = Fixture::new(200, LIST_BODY);
    fixture
        .params
        .insert(String::from("api_key"), json!("sk-1"));
    let query = ContainerListQuery {
        after: None,
        limit: None,
        order: None,
        extra_query: Some(&extra),
    };
    let call = fixture.list_call(query, None);
    container_list_handler(&call, &fixture.source, None, &fixture.hook).unwrap();
    assert_eq!(
        rendered(&fixture.sent()[0]),
        BTreeMap::from([
            (String::from("flag"), String::from("true")),
            (String::from("name"), String::from("x")),
            (String::from("none"), String::new()),
            (String::from("page"), String::from("5")),
        ])
    );
}

#[test]
fn list_sends_an_array_extra_query_as_one_json_text_value() {
    let mut extra = Object::new();
    extra.insert(String::from("tag"), json!(["a", "b"]));
    let fixture = Fixture::new(200, LIST_BODY);
    let query = ContainerListQuery {
        after: None,
        limit: None,
        order: None,
        extra_query: Some(&extra),
    };
    let call = fixture.list_call(query, None);
    container_list_handler(&call, &fixture.source, None, &fixture.hook).unwrap();
    assert_eq!(
        rendered(&fixture.sent()[0]),
        BTreeMap::from([(String::from("tag"), String::from("[\"a\",\"b\"]"))]),
        "httpx repeats the key once per element; the rendered map holds the array's JSON text instead"
    );
}

#[test]
fn list_logs_the_params_before_the_get() {
    let fixture = Fixture::new(202, LIST_BODY);
    let mut extra = Object::new();
    extra.insert(String::from("page"), json!(5));
    let query = ContainerListQuery {
        after: Some("cntr_a"),
        limit: Some(2),
        order: Some("asc"),
        extra_query: Some(&extra),
    };
    let call = fixture.list_call(query, None);
    container_list_handler(&call, &fixture.source, None, &fixture.hook).unwrap();
    assert_eq!(fixture.order(), vec!["log", "send"]);
    let calls = fixture.hook.calls();
    assert_eq!(calls.len(), 1);
    let (input, args) = &calls[0];
    assert_eq!(input, &Json::Text(String::new()));
    let expected = api_base_with_version();
    assert_eq!(args.api_base.as_deref(), Some(expected.as_str()));
    assert_eq!(args.passthrough.len(), 1);
    let Some(Json::Object(params)) = args.passthrough.get("params") else {
        panic!("the pre_call carries the params");
    };
    assert_eq!(
        params.get("after"),
        Some(&Json::Text(String::from("cntr_a")))
    );
    assert_eq!(params.get("limit"), Some(&Json::Text(String::from("2"))));
    assert_eq!(params.get("order"), Some(&Json::Text(String::from("asc"))));
    assert_eq!(
        params.get("page"),
        Some(&Json::Text(String::from("5"))),
        "the record carries every value as the text that goes on the wire"
    );
    assert!(args.complete_input_dict.is_none());
}

#[test]
fn list_overlays_extra_headers_after_validation_and_its_auth_mutation() {
    let mut fixture = Fixture::new(200, LIST_BODY);
    fixture.params = api_key("sk-1");
    let extra = BTreeMap::from([
        (
            String::from("Authorization"),
            String::from("Bearer overlay"),
        ),
        (String::from("X-Extra"), String::from("1")),
    ]);
    fixture.extra_headers = Some(extra.clone());
    let call = fixture.list_call(empty_page(), None);
    container_list_handler(&call, &fixture.source, None, &fixture.hook).unwrap();
    let environment = fixture.config.environment.lock().unwrap().clone();
    assert_eq!(environment[0].0, extra);
    let sent = fixture.sent();
    let headers = &sent[0].headers;
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer overlay")
    );
    assert_eq!(headers.get("X-Extra").map(String::as_str), Some("1"));
}

#[test]
fn list_adds_no_content_type() {
    let mut fixture = Fixture::new(200, LIST_BODY);
    fixture.params = api_key("sk-1");
    let call = fixture.list_call(empty_page(), None);
    container_list_handler(&call, &fixture.source, None, &fixture.hook).unwrap();
    let sent = fixture.sent();
    assert!(!sent[0].headers.contains_key("Content-Type"));
    assert_eq!(
        sent[0].headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-1")
    );
}

#[test]
fn list_sends_through_the_supplied_client_and_builds_none() {
    let fixture = Fixture::new(200, LIST_BODY);
    let supplied = FakeClient::answering(&fixture.events, 200, LIST_BODY);
    let client = BlockingFake(supplied.clone());
    let call = fixture.list_call(empty_page(), None);
    let page =
        container_list_handler(&call, &fixture.source, Some(&client), &fixture.hook).unwrap();
    assert_eq!(page.data[0].id, "cntr_a");
    assert_eq!(supplied.seen().len(), 1);
    assert!(fixture.source.blocking.seen().is_empty());
    assert!(fixture.source.blocking_made.lock().unwrap().is_empty());
}

#[test]
fn list_builds_the_blocking_client_with_the_ssl_verify_param() {
    let mut fixture = Fixture::new(200, LIST_BODY);
    fixture
        .params
        .insert(String::from("ssl_verify"), json!("/etc/ca.pem"));
    let call = fixture.list_call(empty_page(), None);
    container_list_handler(&call, &fixture.source, None, &fixture.hook).unwrap();
    assert_eq!(
        fixture.source.blocking_made.lock().unwrap().clone(),
        vec![Some(SslVerify::CaBundle(String::from("/etc/ca.pem")))]
    );
    assert!(fixture.source.async_made.lock().unwrap().is_empty());
}

#[test]
fn list_maps_a_url_failure_through_the_provider_exception() {
    let fixture = Fixture::failing(url_failure());
    let call = fixture.list_call(empty_page(), None);
    let failure = container_list_handler(&call, &fixture.source, None, &fixture.hook).unwrap_err();
    let provider = provider_error(&failure);
    assert_eq!(provider.status_code, 500);
    assert_eq!(provider.message, "the request URL :: is not valid");
}

#[tokio::test]
async fn list_maps_a_timed_out_get_to_a_synthesized_500() {
    let fixture = Fixture::failing(deadline_failure().await);
    let call = fixture.list_call(empty_page(), None);
    let failure = container_list_handler(&call, &fixture.source, None, &fixture.hook).unwrap_err();
    let provider = provider_error(&failure);
    assert_eq!(
        provider.status_code, 500,
        "a GET timeout is caught as a plain exception, so _handle_error synthesizes the 500"
    );
}

#[test]
fn list_reports_the_error_message_for_an_error_status() {
    let body =
        r#"{"error":{"message":"no such page","type":"invalid_request_error","code":"not_found"}}"#;
    let fixture = Fixture::new(404, body);
    let call = fixture.list_call(empty_page(), None);
    let failure = container_list_handler(&call, &fixture.source, None, &fixture.hook).unwrap_err();
    let provider = provider_error(&failure);
    assert_eq!(provider.status_code, 404);
    assert_eq!(
        provider.message, "no such page",
        "raise_for_error_status reports the error message inside the body"
    );
}

#[test]
fn list_keeps_the_body_text_when_the_error_has_no_message() {
    let fixture = Fixture::new(400, "plain failure");
    let call = fixture.list_call(empty_page(), None);
    let failure = container_list_handler(&call, &fixture.source, None, &fixture.hook).unwrap_err();
    let provider = provider_error(&failure);
    assert_eq!(provider.status_code, 400);
    assert_eq!(provider.message, "plain failure");
}

#[test]
fn list_lets_a_3xx_body_through_to_the_transform() {
    let fixture = Fixture::new(304, LIST_BODY);
    let call = fixture.list_call(empty_page(), None);
    let page = container_list_handler(&call, &fixture.source, None, &fixture.hook).unwrap();
    assert_eq!(page.data[0].id, "cntr_a");
}

#[test]
fn list_rejects_a_bad_param_before_it_asks_for_a_client() {
    let mut fixture = Fixture::new(200, LIST_BODY);
    fixture.params.insert(String::from("api_base"), json!(5));
    let call = fixture.list_call(empty_page(), None);
    let failure = container_list_handler(&call, &fixture.source, None, &fixture.hook).unwrap_err();
    assert!(matches!(failure, ContainerCollectionError::Params(_)));
    assert!(fixture.source.blocking_made.lock().unwrap().is_empty());
    assert!(fixture.sent().is_empty());
}

#[test]
fn list_asks_the_config_for_the_base_with_the_api_base_and_every_param() {
    let mut fixture = Fixture::new(200, LIST_BODY);
    fixture
        .params
        .insert(String::from("api_base"), json!("https://given.test"));
    fixture
        .params
        .insert(String::from("api_key"), json!("sk-9"));
    let call = fixture.list_call(empty_page(), None);
    container_list_handler(&call, &fixture.source, None, &fixture.hook).unwrap();
    assert_eq!(
        fixture.config.bases.lock().unwrap().clone(),
        vec![(
            Some(String::from("https://given.test")),
            fixture.params.clone()
        )]
    );
}

#[tokio::test]
async fn list_async_sends_through_the_async_client() {
    let fixture = Fixture::new(200, LIST_BODY);
    let call = fixture.list_call(all_page(), None);
    let page = acontainer_list_handler(&call, &fixture.source, None, &fixture.hook)
        .await
        .unwrap();
    assert_eq!(page.data[0].id, "cntr_a");
    assert_eq!(
        fixture.source.async_made.lock().unwrap().clone(),
        vec![None]
    );
    assert!(fixture.source.blocking_made.lock().unwrap().is_empty());
    assert_eq!(fixture.sent().len(), 1);
    assert_eq!(fixture.order(), vec!["log", "send"]);
}
