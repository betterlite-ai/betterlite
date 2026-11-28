#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, pending};
use std::time::Duration;

use betterlite_http::proxy::pool::PoolError;
use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_models::decisions::Json;
use betterlite_openai::containers::container_file_http::request::SslVerify;
use betterlite_openai::containers::container_file_http::transport::{
    ClientSource, ContainerMethod,
};
use betterlite_openai::openai_base_config::Object;
use serde_json::json;

use super::tests::{
    BASE, BlockingFake, CREATE_BODY, FakeClient, Fixture, REQUEST_HEADERS, REQUEST_ID,
    RecordingHook, deadline_failure, provider_error, url_failure,
};
use super::{
    CollectionBase, CollectionReply, CollectionRequest, CollectionTransport,
    ContainerCollectionError, ContainerCreateCall, DEFAULT_TIMEOUT, acontainer_create_handler,
    container_create_handler,
};

fn api_key_params(value: &str) -> Object {
    let mut params = Object::new();
    params.insert(String::from("api_key"), json!(value));
    params
}

#[test]
fn create_posts_the_transformed_body_with_the_default_timeout() {
    let mut fixture = Fixture::new(200, CREATE_BODY);
    fixture.params = api_key_params("sk-1");
    fixture.create_params =
        serde_json::from_str(r#"{"expires_after":{"anchor":"last_active_at","minutes":20}}"#)
            .unwrap();
    let call = fixture.create_call("box");
    let object = container_create_handler(&call).unwrap();
    let sent = fixture.sent();
    assert_eq!(sent.len(), 1);
    let request = &sent[0];
    assert_eq!(request.method, ContainerMethod::Post);
    assert_eq!(request.url, BASE);
    assert!(request.query.is_empty(), "the POST sends no params");
    assert_eq!(request.timeout, Some(Duration::from_secs(600)));
    let authorization = request.headers.get("Authorization").map(String::as_str);
    let content_type = request.headers.get("Content-Type").map(String::as_str);
    assert_eq!(authorization, Some("Bearer sk-1"));
    assert_eq!(content_type, Some("application/json"));
    let body: Object = serde_json::from_slice(request.body.as_deref().unwrap()).unwrap();
    let expected: Object = serde_json::from_str(
        r#"{"name":"box","expires_after":{"anchor":"last_active_at","minutes":20}}"#,
    )
    .unwrap();
    assert_eq!(body, expected);
    assert_eq!(object.id, "cntr_abc");
    assert_eq!(object.name.as_deref(), Some("box"));
    assert_eq!(object.status, "running");
    assert_eq!(
        fixture.config.creates.lock().unwrap().clone(),
        vec![(
            String::from("box"),
            fixture.create_params.clone(),
            request.headers.clone(),
        )]
    );
}

#[test]
fn create_validates_the_environment_before_the_content_type_goes_in() {
    let mut fixture = Fixture::new(200, CREATE_BODY);
    fixture.params = api_key_params("sk-1");
    fixture.extra_headers = Some(BTreeMap::from([(
        String::from("X-Extra"),
        String::from("1"),
    )]));
    let call = fixture.create_call("box");
    container_create_handler(&call).unwrap();
    let environment = fixture.config.environment.lock().unwrap().clone();
    assert_eq!(environment.len(), 1);
    let (seen, key) = &environment[0];
    assert!(
        !seen.contains_key("Content-Type"),
        "the JSON content type is inserted after validate_environment"
    );
    assert_eq!(key.as_deref(), Some("sk-1"));
    assert_eq!(
        fixture.sent()[0]
            .headers
            .get("Content-Type")
            .map(String::as_str),
        Some("application/json")
    );
}

#[test]
fn create_overlays_extra_headers_after_validation_and_its_auth_mutation() {
    let mut fixture = Fixture::new(200, CREATE_BODY);
    fixture.params = api_key_params("sk-1");
    let extra = BTreeMap::from([
        (
            String::from("Authorization"),
            String::from("Bearer overlay"),
        ),
        (String::from("X-Extra"), String::from("1")),
    ]);
    fixture.extra_headers = Some(extra.clone());
    let call = fixture.create_call("box");
    container_create_handler(&call).unwrap();
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
fn create_lets_an_extra_content_type_win_over_the_json_one() {
    let mut fixture = Fixture::new(200, CREATE_BODY);
    fixture.extra_headers = Some(BTreeMap::from([(
        String::from("Content-Type"),
        String::from("text/custom"),
    )]));
    let call = fixture.create_call("box");
    container_create_handler(&call).unwrap();
    let sent = fixture.sent();
    assert_eq!(
        sent[0].headers.get("Content-Type").map(String::as_str),
        Some("text/custom")
    );
}

#[test]
fn create_passes_the_given_timeout_to_the_post() {
    let fixture = Fixture::new(200, CREATE_BODY);
    let mut call = fixture.create_call("box");
    call.base.timeout = Some(Duration::from_secs(5));
    container_create_handler(&call).unwrap();
    assert_eq!(fixture.sent()[0].timeout, Some(Duration::from_secs(5)));
}

#[test]
fn create_logs_the_name_and_the_body_before_the_post() {
    let mut fixture = Fixture::new(201, CREATE_BODY);
    fixture.params = api_key_params("sk-1");
    let call = fixture.create_call("box");
    let object = container_create_handler(&call).unwrap();
    assert_eq!(object.id, "cntr_abc");
    assert_eq!(fixture.order(), vec!["log", "send"]);
    let calls = fixture.hook.calls();
    assert_eq!(calls.len(), 1);
    let (input, args) = &calls[0];
    assert_eq!(input, &Json::Text(String::from("box")));
    assert_eq!(args.api_base.as_deref(), Some(BASE));
    assert!(args.passthrough.is_empty());
    let complete = args.complete_input_dict.clone().unwrap();
    let RawBody::Object(fields) = complete else {
        panic!("the complete input is the JSON object");
    };
    assert_eq!(fields.get("name"), Some(&Json::Text(String::from("box"))));
    assert_eq!(
        fields.get("expires_after"),
        None,
        "the fixture sends no optional params"
    );
    let logged = args.headers.clone().unwrap();
    assert_eq!(
        logged.get("Content-Type"),
        Some(&Json::Text(String::from("application/json")))
    );
}

#[test]
fn create_sends_nothing_when_the_pre_call_hook_fails() {
    let mut fixture = Fixture::new(200, CREATE_BODY);
    fixture.hook = RecordingHook::failing(&fixture.events);
    let call = fixture.create_call("box");
    let failure = container_create_handler(&call).unwrap_err();
    assert!(matches!(failure, ContainerCollectionError::Logging(_)));
    assert!(fixture.sent().is_empty());
    assert_eq!(fixture.order(), vec!["log"]);
}

#[test]
fn create_sends_through_the_supplied_client_and_builds_none() {
    let fixture = Fixture::new(200, CREATE_BODY);
    let supplied = FakeClient::answering(&fixture.events, 200, CREATE_BODY);
    let client = BlockingFake(supplied.clone());
    let mut call = fixture.create_call("box");
    call.base.blocking = Some(&client);
    let object = container_create_handler(&call).unwrap();
    assert_eq!(object.id, "cntr_abc");
    assert_eq!(supplied.seen().len(), 1);
    assert!(fixture.source.blocking.seen().is_empty());
    assert!(fixture.source.blocking_made.lock().unwrap().is_empty());
}

#[test]
fn create_builds_the_blocking_client_with_the_ssl_verify_param() {
    let mut fixture = Fixture::new(200, CREATE_BODY);
    fixture
        .params
        .insert(String::from("ssl_verify"), json!(false));
    let call = fixture.create_call("box");
    container_create_handler(&call).unwrap();
    assert_eq!(
        fixture.source.blocking_made.lock().unwrap().clone(),
        vec![Some(SslVerify::Enabled(false))]
    );
    assert!(fixture.source.async_made.lock().unwrap().is_empty());
}

#[test]
fn create_maps_a_url_failure_through_the_provider_exception() {
    let fixture = Fixture::failing(url_failure());
    let call = fixture.create_call("box");
    let failure = container_create_handler(&call).unwrap_err();
    let provider = provider_error(&failure);
    assert_eq!(provider.status_code, 500);
    assert_eq!(provider.message, "the request URL :: is not valid");
    assert!(provider.headers.is_empty());
}

#[tokio::test]
async fn create_maps_a_timed_out_post_to_408() {
    let fixture = Fixture::failing(deadline_failure().await);
    let call = fixture.create_call("box");
    let failure = container_create_handler(&call).unwrap_err();
    let provider = provider_error(&failure);
    assert_eq!(provider.status_code, 408);
}

#[test]
fn create_reports_the_whole_body_for_an_error_status() {
    let body = r#"{"error":{"message":"bad name","type":"invalid_request_error","param":"name","code":"x"}}"#;
    let fixture = Fixture::new(400, body);
    let call = fixture.create_call("box");
    let failure = container_create_handler(&call).unwrap_err();
    let provider = provider_error(&failure);
    assert_eq!(provider.status_code, 400);
    assert_eq!(
        provider.message, body,
        "a POST error keeps the whole body text, not the error message inside it"
    );
    assert_eq!(
        provider.headers.get(REQUEST_HEADERS).map(String::as_str),
        Some(REQUEST_ID)
    );
}

#[test]
fn create_lets_a_299_reply_through_to_the_transform() {
    let fixture = Fixture::new(299, CREATE_BODY);
    let call = fixture.create_call("box");
    let object = container_create_handler(&call).unwrap();
    assert_eq!(
        object.id, "cntr_abc",
        "299 is the last 2xx status, so raise_for_status accepts its body"
    );
}

#[test]
fn create_maps_a_1xx_reply_through_the_provider_exception() {
    let fixture = Fixture::new(199, CREATE_BODY);
    let call = fixture.create_call("box");
    let failure = container_create_handler(&call).unwrap_err();
    let provider = provider_error(&failure);
    assert_eq!(
        provider.status_code, 199,
        "raise_for_status refuses every non-2xx status, informational ones included"
    );
}

#[test]
fn create_reports_the_size_of_a_non_utf8_error_body() {
    let fixture = Fixture::with_bytes(400, vec![0xff, 0xfe]);
    let call = fixture.create_call("box");
    let failure = container_create_handler(&call).unwrap_err();
    let provider = provider_error(&failure);
    assert_eq!(provider.status_code, 400);
    assert_eq!(
        provider.message, "2 bytes that are not UTF-8",
        "the body's bytes are reported by size where httpx would decode replacement characters"
    );
}

#[test]
fn create_maps_a_3xx_reply_through_the_provider_exception() {
    let fixture = Fixture::new(300, CREATE_BODY);
    let call = fixture.create_call("box");
    let failure = container_create_handler(&call).unwrap_err();
    let provider = provider_error(&failure);
    assert_eq!(
        provider.status_code, 300,
        "the POST calls raise_for_status, so a 3xx reply is an error like any other non-2xx"
    );
    assert_eq!(
        provider.message, CREATE_BODY,
        "the exception carries the whole reply body as its message"
    );
}

#[test]
fn create_rejects_a_bad_param_before_it_asks_for_a_client() {
    let mut fixture = Fixture::new(200, CREATE_BODY);
    fixture.params.insert(String::from("api_key"), json!(5_i32));
    let call = fixture.create_call("box");
    let failure = container_create_handler(&call).unwrap_err();
    assert!(matches!(failure, ContainerCollectionError::Params(_)));
    assert!(fixture.source.blocking_made.lock().unwrap().is_empty());
    assert!(fixture.sent().is_empty());
}

#[test]
fn create_asks_the_config_for_the_base_with_the_api_base_and_every_param() {
    let mut fixture = Fixture::new(200, CREATE_BODY);
    fixture
        .params
        .insert(String::from("api_base"), json!("https://given.test"));
    fixture
        .params
        .insert(String::from("api_key"), json!("sk-9"));
    let call = fixture.create_call("box");
    container_create_handler(&call).unwrap();
    assert_eq!(
        fixture.config.bases.lock().unwrap().clone(),
        vec![(
            Some(String::from("https://given.test")),
            fixture.params.clone()
        )]
    );
}

#[tokio::test]
async fn create_async_sends_through_the_async_client() {
    let mut fixture = Fixture::new(200, CREATE_BODY);
    fixture.params = api_key_params("sk-1");
    let call = fixture.create_call("box");
    let object = acontainer_create_handler(&call).await.unwrap();
    assert_eq!(object.id, "cntr_abc");
    assert_eq!(
        fixture.source.async_made.lock().unwrap().clone(),
        vec![None]
    );
    assert!(fixture.source.blocking_made.lock().unwrap().is_empty());
    assert_eq!(fixture.sent().len(), 1);
    assert_eq!(fixture.order(), vec!["log", "send"]);
    assert_eq!(fixture.sent()[0].timeout, Some(DEFAULT_TIMEOUT));
}

// Holds its answer forever, so the blocking handler's one poll finds the send still pending
struct HeldClient;

impl CollectionTransport for HeldClient {
    fn send(&self, _request: &CollectionRequest) -> impl Future<Output = CollectionReply> + Send {
        pending()
    }
}

struct HeldSource;

impl ClientSource for HeldSource {
    type Blocking = HeldClient;
    type Asynchronous = HeldClient;

    fn blocking_client(&self, _ssl_verify: Option<&SslVerify>) -> Result<HeldClient, PoolError> {
        Ok(HeldClient)
    }

    fn async_client(&self, _ssl_verify: Option<&SslVerify>) -> Result<HeldClient, PoolError> {
        Ok(HeldClient)
    }
}

#[test]
fn create_reports_a_client_that_never_answers_as_suspended() {
    let fixture = Fixture::new(200, CREATE_BODY);
    let source = HeldSource;
    let call = ContainerCreateCall {
        base: CollectionBase {
            config: &fixture.config,
            litellm_params: &fixture.params,
            logging: &fixture.logging,
            extra_headers: None,
            timeout: None,
            source: &source,
            blocking: None,
            asynchronous: None,
            hook: &fixture.hook,
        },
        name: "box",
        params: &fixture.create_params,
    };
    let failure = container_create_handler(&call).unwrap_err();
    assert!(
        matches!(failure, ContainerCollectionError::Suspended),
        "a blocking caller gets an error instead of waiting on a client that has not answered"
    );
}
