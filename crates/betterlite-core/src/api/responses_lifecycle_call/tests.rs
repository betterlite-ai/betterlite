#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_context::resource_config_selection::responses::ResponsesConfigTag;
use betterlite_error::envelope::WireField;
use betterlite_models::decisions::Json;
use betterlite_openai::response_resources::request::InputItemsOrder;
use betterlite_openai::response_resources::response::TypedReply;
use serde_json::Number;

use self::support::{
    Answer, DEFAULT_TIMEOUT, Fixture, Recorded, caller_headers, input_items, json, map, retrieve,
};
use super::error::{LifecycleError, LifecycleFailure};
use super::input::{
    LifecyclePhases, LifecycleTimeout, ResponsesInputItemsCall, ResponsesRetrieveCall,
};
use crate::api::response_id_codecs::response::{build_response_id, decode_response_id};
use crate::api::responses_read_http::error::ReadError;
use crate::api::responses_read_http::service::ClientKind;

mod support;

const BODY: &str =
    r#"{"id":"resp_a","created_at":9,"status":"completed","model":"gpt-4o","output":[]}"#;
const ITEMS: &str =
    r#"{"object":"list","data":[{"id":"item_a","type":"message"}],"has_more":false}"#;
const DOD_404: &str = r#"{"error": {"message": "gone", "type": "invalid_request_error", "param": "response_id", "code": "not_found"}}"#;
const PLAIN_404: &str = r#"{"error": {"message": "gone"}}"#;
const CALL: &str = r#"{"response_id":"resp_a","custom_llm_provider":"openai","api_base":"http://mock.test/v1","api_key":"sk-provider","litellm_call_id":"call-a","litellm_metadata":{"model_info":{"id":"deployment-a"}}}"#;
const NO_CALL_ID: &str = r#"{"response_id":"resp_a","custom_llm_provider":"openai","api_base":"http://mock.test/v1","api_key":"sk-provider","litellm_metadata":{"model_info":{"id":"deployment-a"}}}"#;
const MISSING_PROVIDER: &str = r#"{"response_id":"resp_a"}"#;
const ANTHROPIC: &str = r#"{"response_id":"resp_a","custom_llm_provider":"anthropic"}"#;

// checks the rewritten id and the response's own fields, and hands the reply back for the header checks
fn retrieved(reply: TypedReply, model_id: &str, response_id: &str) -> TypedReply {
    assert_eq!(
        reply.response.id,
        build_response_id(Some("openai"), Some(model_id), response_id)
    );
    let decoded = decode_response_id(&reply.response.id);
    assert_eq!(
        decoded.custom_llm_provider.flatten().as_deref(),
        Some("openai")
    );
    assert_eq!(decoded.model_id.flatten().as_deref(), Some(model_id));
    assert_eq!(decoded.response_id.as_deref(), Some(response_id));
    assert_eq!(reply.response.created_at, 9_i64);
    assert_eq!(reply.response.status.as_deref(), Some("completed"));
    assert_eq!(reply.response.model.as_deref(), Some("gpt-4o"));
    reply
}

// checks that the exception's flat fields read as null rather than absent, and hands it back
fn null_fields(failure: Box<LifecycleFailure>) -> Box<LifecycleFailure> {
    assert_eq!(failure.kind, WireField::Null);
    assert_eq!(failure.param, WireField::Null);
    assert_eq!(failure.code, WireField::Null);
    failure
}

// the encoded id reaches the provider decoded, and the rewrite encodes the answered id exactly once
#[tokio::test]
async fn retrieve_routes_encoded_id() {
    let mut fixture = Fixture::new(vec![
        Answer::Retrieved {
            body: BODY,
            header: Some(("x-request-id", "req_a")),
        },
        Answer::Retrieved {
            body: BODY,
            header: Some(("x-request-id", "req_b")),
        },
        Answer::Retrieved {
            body: BODY,
            header: None,
        },
    ]);
    let call = encoded_retrieve();
    assert_encoded_retrievals(&mut fixture, &call).await;
    assert_raw_retrieval(&mut fixture).await;
    assert_retrieval_records(&fixture.service.executor.records());
}

// the encoded id names openai in its payload while the explicit provider says anthropic; the id wins
fn encoded_retrieve() -> ResponsesRetrieveCall {
    let mut call = retrieve(CALL);
    call.identity.response_id = build_response_id(Some("openai"), Some("deployment-a"), "resp/a");
    call.identity.custom_llm_provider = Some(String::from("anthropic"));
    call
}

async fn assert_encoded_retrievals(fixture: &mut Fixture, call: &ResponsesRetrieveCall) {
    let reply = retrieved(
        fixture
            .retrieve(call)
            .await
            .expect("the encoded retrieve succeeds"),
        "deployment-a",
        "resp/a",
    );
    assert_eq!(
        reply.headers.get("x-request-id").map(String::as_str),
        Some("req_a")
    );

    let awaited = retrieved(
        fixture
            .aretrieve(call)
            .await
            .expect("the awaited retrieve succeeds"),
        "deployment-a",
        "resp/a",
    );
    assert_eq!(
        awaited.headers.get("x-request-id").map(String::as_str),
        Some("req_b")
    );
}

async fn assert_raw_retrieval(fixture: &mut Fixture) {
    let plain = retrieved(
        fixture
            .retrieve(&retrieve(CALL))
            .await
            .expect("the raw retrieve succeeds"),
        "deployment-a",
        "resp_a",
    );
    assert!(plain.headers.is_empty());
}

fn assert_retrieval_records(records: &[Recorded]) {
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].response_id, "resp/a");
    assert_eq!(records[0].tag, ResponsesConfigTag::OpenAi);
    assert_eq!(records[0].api_base.as_deref(), Some("http://mock.test/v1"));
    assert_eq!(records[0].api_key.as_deref(), Some("sk-provider"));
    assert_eq!(records[0].mode, ClientKind::Sync);
    assert!(records[0].query.is_none());
    assert!(records[0].extra_body.is_none());
    assert_eq!(records[1].response_id, "resp/a");
    assert_eq!(records[1].mode, ClientKind::Asynchronous);
    assert_eq!(
        records[1].logged_litellm.get("litellm_call_id"),
        Some(&Json::Text(String::from("call-a")))
    );
    assert_eq!(records[2].response_id, "resp_a");
    assert_eq!(records[2].tag, ResponsesConfigTag::OpenAi);
    assert_eq!(records[2].mode, ClientKind::Sync);
}

// the five SDK pagination arguments travel as a query, and an absent limit and order keep the SDK defaults
#[tokio::test]
async fn input_items_sdk_pagination() {
    let mut fixture = Fixture::new(vec![
        Answer::Items(ITEMS),
        Answer::Items(ITEMS),
        Answer::Items(ITEMS),
        Answer::Items(ITEMS),
    ]);
    let default_page = fixture
        .list(&input_items(CALL))
        .await
        .expect("the default listing succeeds");
    assert_eq!(default_page, json(ITEMS));
    let explicit_page = fixture
        .list(&explicit_listing())
        .await
        .expect("the explicit listing succeeds");
    assert_eq!(explicit_page, json(ITEMS));
    let empty_page = fixture
        .list(&empty_listing())
        .await
        .expect("the empty listing succeeds");
    assert_eq!(empty_page, json(ITEMS));
    let awaited = fixture
        .alist(&input_items(CALL))
        .await
        .expect("the awaited listing succeeds");
    assert_eq!(awaited, json(ITEMS));
    let records = fixture.service.executor.records();
    assert_pagination_queries(&records);
    assert_listing_records(&records);
}

// after and before travel as given, and an explicit limit and order replace the defaults
fn explicit_listing() -> ResponsesInputItemsCall {
    let mut explicit = input_items(CALL);
    explicit.after = Some(String::new());
    explicit.before = Some(String::from("item_b"));
    explicit.include = Some(vec![String::from("message.input_image.image_url")]);
    explicit.limit = Some(Some(2));
    explicit.order = Some(Some(InputItemsOrder::Asc));
    explicit
}

// an empty include list stays an empty list, and an explicit null limit and order are not the defaults
fn empty_listing() -> ResponsesInputItemsCall {
    let mut empty = input_items(CALL);
    empty.include = Some(Vec::new());
    empty.limit = Some(None);
    empty.order = Some(None);
    empty
}

fn assert_pagination_queries(records: &[Recorded]) {
    let Some(default_query) = &records[0].query else {
        panic!("the default listing sends a query");
    };
    assert_eq!(default_query.after, None);
    assert_eq!(default_query.before, None);
    assert_eq!(default_query.include, None);
    assert_eq!(default_query.limit, Some(20));
    assert_eq!(default_query.order, Some(InputItemsOrder::Desc));
    let Some(explicit_query) = &records[1].query else {
        panic!("the explicit listing sends a query");
    };
    assert_eq!(explicit_query.after.as_deref(), Some(""));
    assert_eq!(explicit_query.before.as_deref(), Some("item_b"));
    assert_eq!(
        explicit_query.include,
        Some(vec![String::from("message.input_image.image_url")])
    );
    assert_eq!(explicit_query.limit, Some(2));
    assert_eq!(explicit_query.order, Some(InputItemsOrder::Asc));
    let Some(empty_query) = &records[2].query else {
        panic!("the empty listing sends a query");
    };
    assert_eq!(empty_query.include, Some(Vec::new()));
    assert_eq!(empty_query.limit, None);
    assert_eq!(empty_query.order, None);
}

fn assert_listing_records(records: &[Recorded]) {
    assert_eq!(records.len(), 4);
    assert_eq!(records[3].query, records[0].query);
    assert_eq!(records[3].mode, ClientKind::Asynchronous);
    assert_eq!(records[3].response_id, "resp_a");
    assert!(
        records
            .iter()
            .all(|record| record.tag == ResponsesConfigTag::OpenAi)
    );
    assert!(records.iter().all(|record| record.response_id == "resp_a"));
}

// extra_query never reaches the read seam, the timeout default follows the configured value, and only a JSON true flag is async
#[tokio::test]
async fn read_ignored_extras_and_context() {
    let mut fixture = extras_fixture();
    let headers = assert_ignored_extras(&mut fixture).await;
    assert_call_variants(&mut fixture).await;
    assert_read_records(&fixture.service.executor.records(), &headers);
    assert_eq!(fixture.state().model, "responses");
    assert_eq!(
        fixture.state().custom_llm_provider.as_deref(),
        Some("openai")
    );
    assert_injected_timeout().await;
}

// eleven queued answers, one per read the test makes, in the order it makes them
fn extras_fixture() -> Fixture {
    let answer = Answer::Retrieved {
        body: BODY,
        header: None,
    };
    let mut answers = vec![answer; 8];
    answers.extend([Answer::Items(ITEMS), Answer::Items(ITEMS)]);
    answers.push(Answer::Retrieved {
        body: BODY,
        header: None,
    });
    Fixture::new(answers)
}

// the caller's headers and body reach the seam, while extra_query stays off the query
async fn assert_ignored_extras(fixture: &mut Fixture) -> BTreeMap<String, String> {
    let mut extras = retrieve(CALL);
    let headers = caller_headers(&[("X-Trace", "trace-a")]);
    assert_eq!(headers.get("X-Trace").map(String::as_str), Some("trace-a"));
    extras.identity.extra_headers = Some(headers.clone());
    extras.extra_query = Some(map(r#"{"after":"ignored"}"#));
    extras.extra_body = Some(map(r#"{"ignored":true}"#));
    extras.identity.timeout = Some(LifecycleTimeout::Seconds(Number::from(7_i32)));
    extras.identity.metadata = Some(map(r#"{"request_id":"r1"}"#));
    drop(
        fixture
            .retrieve(&extras)
            .await
            .expect("the extras retrieve succeeds"),
    );
    headers
}

// a read the test expects to succeed, dropped because the fixture records what the service handed it
async fn expect_retrieve(fixture: &mut Fixture, call: &ResponsesRetrieveCall) {
    drop(fixture.retrieve(call).await.expect("the retrieve succeeds"));
}

async fn expect_list(fixture: &mut Fixture, call: &ResponsesInputItemsCall) {
    drop(fixture.list(call).await.expect("the listing succeeds"));
}

// an httpx.Timeout's phases, with only the two the read cares about named
fn phases_with(connect: Option<Number>, read: Option<Number>) -> LifecycleTimeout {
    LifecycleTimeout::Phases(Box::new(LifecyclePhases {
        connect,
        read,
        write: None,
        pool: None,
    }))
}

// absent, zero and phase timeouts follow the SDK defaults, and only a JSON true flag selects the async client
async fn assert_call_variants(fixture: &mut Fixture) {
    expect_retrieve(fixture, &retrieve(NO_CALL_ID)).await;
    let mut zero = retrieve(NO_CALL_ID);
    zero.identity.timeout = Some(LifecycleTimeout::Seconds(Number::from(0_i32)));
    expect_retrieve(fixture, &zero).await;
    let mut read_phase = retrieve(NO_CALL_ID);
    read_phase.identity.timeout = Some(phases_with(None, Some(Number::from(9_i32))));
    expect_retrieve(fixture, &read_phase).await;
    let mut no_read = retrieve(NO_CALL_ID);
    no_read.identity.timeout = Some(phases_with(Some(Number::from(3_i32)), None));
    expect_retrieve(fixture, &no_read).await;
    let mut number_flag = retrieve(NO_CALL_ID);
    number_flag.aget_responses = Some(json("1"));
    expect_retrieve(fixture, &number_flag).await;
    let mut text_flag = retrieve(NO_CALL_ID);
    text_flag.aget_responses = Some(Json::Text(String::from("true")));
    expect_retrieve(fixture, &text_flag).await;
    let mut bool_flag = retrieve(NO_CALL_ID);
    bool_flag.aget_responses = Some(Json::Bool(true));
    expect_retrieve(fixture, &bool_flag).await;
    let mut items_number = input_items(NO_CALL_ID);
    items_number.alist_input_items = Some(json("1"));
    expect_list(fixture, &items_number).await;
    let mut items_bool = input_items(NO_CALL_ID);
    items_bool.alist_input_items = Some(Json::Bool(true));
    expect_list(fixture, &items_bool).await;
    let mut zero_read = retrieve(NO_CALL_ID);
    zero_read.identity.timeout = Some(phases_with(None, Some(Number::from(0_i32))));
    expect_retrieve(fixture, &zero_read).await;
}

fn assert_read_records(records: &[Recorded], headers: &BTreeMap<String, String>) {
    assert_eq!(records.len(), 11);
    assert_extras_record(&records[0], headers);
    assert_timeout_records(records);
    assert_mode_records(records);
}

fn assert_extras_record(record: &Recorded, headers: &BTreeMap<String, String>) {
    assert_eq!(record.extra_headers.as_ref(), Some(headers));
    assert_eq!(record.extra_body, Some(map(r#"{"ignored":true}"#)));
    assert!(record.query.is_none());
    assert_eq!(record.timeout, Some(Duration::from_secs(7)));
    assert_eq!(record.mode, ClientKind::Sync);
    assert_eq!(record.logged_optional, map(r#"{"response_id":"resp_a"}"#));
    assert_eq!(
        record.logged_litellm.get("litellm_call_id"),
        Some(&Json::Text(String::from("call-a")))
    );
    assert_eq!(
        record.logged_litellm.get("metadata"),
        Some(&Json::Object(map(r#"{"request_id":"r1"}"#)))
    );
    assert_eq!(
        record.logged_litellm.get("litellm_metadata"),
        Some(&Json::Object(map(
            r#"{"model_info":{"id":"deployment-a"}}"#
        )))
    );
}

fn assert_timeout_records(records: &[Recorded]) {
    assert_eq!(records[1].timeout, Some(DEFAULT_TIMEOUT));
    assert_eq!(
        records[1].logged_litellm.get("litellm_call_id"),
        Some(&Json::Null)
    );
    assert_eq!(records[2].timeout, Some(DEFAULT_TIMEOUT));
    assert_eq!(records[3].timeout, Some(Duration::from_secs(9)));
    assert_eq!(records[4].timeout, Some(DEFAULT_TIMEOUT));
    assert_eq!(records[10].timeout, Some(DEFAULT_TIMEOUT));
    assert_eq!(records[10].mode, ClientKind::Sync);
    assert!(records[10].query.is_none());
    assert!(records.iter().take(8).all(|record| record.query.is_none()));
}

fn assert_mode_records(records: &[Recorded]) {
    assert_eq!(records[5].mode, ClientKind::Sync);
    assert_eq!(records[6].mode, ClientKind::Sync);
    assert_eq!(records[7].mode, ClientKind::Asynchronous);
    assert_eq!(
        records[7].logged_optional,
        map(r#"{"response_id":"resp_a"}"#)
    );
    assert_eq!(records[8].mode, ClientKind::Sync);
    assert_eq!(records[9].mode, ClientKind::Asynchronous);
}

// the injected default replaces the imported REQUEST_TIMEOUT when the caller passes no timeout
async fn assert_injected_timeout() {
    let mut injected = Fixture::with_timeout(
        vec![Answer::Retrieved {
            body: BODY,
            header: None,
        }],
        Duration::from_secs(17),
    );
    drop(
        injected
            .retrieve(&retrieve(NO_CALL_ID))
            .await
            .expect("the injected-default retrieve succeeds"),
    );
    assert_eq!(
        injected.service.executor.records()[0].timeout,
        Some(Duration::from_secs(17))
    );
}

// every locally raised and provider error keeps its class, status and flat fields, and the source cause survives
#[tokio::test]
async fn read_provider_errors() {
    let mut fixture = error_fixture();
    assert_missing_provider(&mut fixture).await;
    assert_unsupported_providers(&mut fixture).await;
    assert!(fixture.service.executor.records().is_empty());
    assert_dod_failure(&mut fixture).await;
    assert_plain_failure(&mut fixture).await;
    assert_server_failure(&mut fixture).await;
    assert_auth_failure(&mut fixture).await;
    assert_eq!(fixture.service.executor.records().len(), 4);
}

fn error_fixture() -> Fixture {
    Fixture::new(vec![
        Answer::Provider {
            status: 404,
            message: DOD_404,
        },
        Answer::Provider {
            status: 404,
            message: PLAIN_404,
        },
        Answer::Provider {
            status: 500,
            message: "not json",
        },
        Answer::Provider {
            status: 401,
            message: "unauthorized",
        },
    ])
}

async fn assert_missing_provider(fixture: &mut Fixture) {
    let missing = null_fields(
        fixture
            .retrieve(&retrieve(MISSING_PROVIDER))
            .await
            .expect_err("the missing provider fails before HTTP"),
    );
    assert_eq!(missing.class, "APIConnectionError");
    assert_eq!(missing.status, 500);
    assert_eq!(
        missing.message,
        "Exception - custom_llm_provider is required but passed as None"
    );
    assert_eq!(
        missing.error.to_string(),
        "custom_llm_provider is required but passed as None"
    );

    let awaited = null_fields(
        fixture
            .aretrieve(&retrieve(MISSING_PROVIDER))
            .await
            .expect_err("the awaited missing provider fails the same way"),
    );
    assert_eq!(awaited.class, missing.class);
    assert_eq!(awaited.status, missing.status);
    assert_eq!(awaited.message, missing.message);
}

async fn assert_unsupported_providers(fixture: &mut Fixture) {
    let unsupported = null_fields(
        fixture
            .retrieve(&retrieve(ANTHROPIC))
            .await
            .expect_err("the unsupported provider fails before HTTP"),
    );
    assert_eq!(unsupported.class, "APIConnectionError");
    assert_eq!(unsupported.status, 500);
    assert_eq!(
        unsupported.message,
        "AnthropicException - GET responses is not supported for anthropic"
    );
    let unsupported_items = fixture
        .list(&input_items(ANTHROPIC))
        .await
        .expect_err("the unsupported listing fails before HTTP");
    assert_eq!(
        unsupported_items.message,
        "AnthropicException - list_input_items is not supported for anthropic"
    );
    let unsupported_awaited = fixture
        .alist(&input_items(ANTHROPIC))
        .await
        .expect_err("the awaited unsupported listing fails before HTTP");
    assert_eq!(unsupported_awaited.class, unsupported_items.class);
    assert_eq!(unsupported_awaited.message, unsupported_items.message);
}

async fn assert_dod_failure(fixture: &mut Fixture) {
    let bad = null_fields(
        fixture
            .retrieve(&retrieve(CALL))
            .await
            .expect_err("the 404 fails"),
    );
    assert_eq!(bad.class, "BadRequestError");
    assert_eq!(bad.status, 404);
    assert_eq!(bad.message, format!("OpenAIException - {DOD_404}"));
    let LifecycleError::Read(error) = &bad.error else {
        panic!("the provider cause is kept, got {:?}", bad.error);
    };
    let ReadError::Provider(source) = &**error else {
        panic!("the provider cause is kept, got {:?}", bad.error);
    };
    assert_eq!(source.status_code, 404);
    assert_eq!(source.message, DOD_404);
}

// the failed read, with the class, status and flat fields checked to read as null rather than absent
async fn expect_failure(fixture: &mut Fixture, reason: &str) -> Box<LifecycleFailure> {
    null_fields(fixture.retrieve(&retrieve(CALL)).await.expect_err(reason))
}

// the class, status and message an SDK exception carries
fn assert_sdk_failure(failure: &LifecycleFailure, class: &str, status: u16, message: &str) {
    assert_eq!((failure.class, failure.status), (class, status));
    assert_eq!(failure.message, message);
}

async fn assert_plain_failure(fixture: &mut Fixture) {
    let plain = expect_failure(fixture, "the plain 404 fails").await;
    let message = format!("NotFoundError: OpenAIException - {PLAIN_404}");
    assert_sdk_failure(&plain, "NotFoundError", 404, &message);
}

async fn assert_server_failure(fixture: &mut Fixture) {
    let server = fixture
        .retrieve(&retrieve(CALL))
        .await
        .expect_err("the 500 fails");
    assert_eq!(server.class, "InternalServerError");
    assert_eq!(server.status, 500);
    assert_eq!(
        server.kind,
        WireField::Text(String::from("internal_server_error"))
    );
    assert_eq!(server.param, WireField::Null);
    assert_eq!(server.code, WireField::Null);
    assert_eq!(
        server.message,
        "InternalServerError: OpenAIException - not json"
    );
}

async fn assert_auth_failure(fixture: &mut Fixture) {
    let auth = expect_failure(fixture, "the 401 fails").await;
    let message = "AuthenticationError: OpenAIException - unauthorized";
    assert_sdk_failure(&auth, "AuthenticationError", 401, message);
}
