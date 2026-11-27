use std::time::Duration;

use betterlite_error::envelope::WireField;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::support::{
    FILE_ID, Fixture, REQUEST_TIMEOUT, Spy, VECTOR_STORE_ID, Which, attributes, attributes_json,
    call, deleted, recorded, success,
};
use crate::api::vector_store_http_write::service::{ClientKind, Operation};

// the mapped prefix and handler line an expired deadline renders
const TIMEOUT_PREFIX: &str =
    "Timeout Error: OpenAIException - litellm.Timeout: Connection timed out";
const DELETE_PREFIX: &str = "InternalServerError: OpenAIException - timed out";

#[rstest]
#[case::absent(None, 6000.0_f64, Duration::from_secs(6000))]
#[case::default_zero(Some(0.0_f64), 6000.0_f64, Duration::from_secs(6000))]
#[case::explicit(Some(7.0_f64), 6000.0_f64, Duration::from_secs(7))]
#[case::environment_absent(None, 12.0_f64, Duration::from_secs(12))]
#[case::environment_zero(Some(0.0_f64), 12.0_f64, Duration::from_secs(12))]
#[case::environment_explicit(Some(7.0_f64), 12.0_f64, Duration::from_secs(7))]
// a zero effective timeout is a deadline already due, not one httpx refuses to schedule
#[case::zero_effective(Some(0.0_f64), 0.0_f64, Duration::ZERO)]
// httpx never expires a deadline it cannot schedule, so these keep the request alive
#[case::over_range(Some(1e300_f64), 6000.0_f64, Duration::MAX)]
#[case::environment_over_range(None, 1e300_f64, Duration::MAX)]
#[case::not_a_number(Some(f64::NAN), 6000.0_f64, Duration::MAX)]
#[case::infinite(Some(f64::INFINITY), 6000.0_f64, Duration::MAX)]
#[tokio::test]
async fn write_timeout(
    #[case] requested: Option<f64>,
    #[case] request_timeout_seconds: f64,
    #[case] expected: Duration,
) {
    let spy = Spy::replying(success());
    let mut fixture = Fixture::new();
    fixture.arguments.timeout = requested;
    let seen = recorded(Which::Update, &spy, &fixture, request_timeout_seconds).await;
    assert_eq!(seen.timeout, expected);
}

// A negative deadline expires in httpx before the send: the catches raise 408, and the async delete's missing catch gets a 500
#[rstest]
#[case::update(Which::Update, "Timeout", 408_u16, WireField::Null, TIMEOUT_PREFIX)]
#[case::aupdate(Which::Aupdate, "Timeout", 408_u16, WireField::Null, TIMEOUT_PREFIX)]
#[case::delete(Which::Delete, "Timeout", 408_u16, WireField::Null, TIMEOUT_PREFIX)]
#[case::adelete(
    Which::Adelete,
    "InternalServerError",
    500_u16,
    WireField::Text(String::from("internal_server_error")),
    DELETE_PREFIX
)]
#[tokio::test]
async fn a_negative_timeout_expires_before_the_send(
    #[case] which: Which,
    #[case] class: &str,
    #[case] status: u16,
    #[case] kind: WireField,
    #[case] prefix: &str,
) {
    let spy = Spy::replying(success());
    let mut fixture = Fixture::new();
    fixture.arguments.timeout = Some(-1.0_f64);
    let failure = call(which, &spy, &fixture, REQUEST_TIMEOUT)
        .await
        .expect_err("the deadline expired");
    let raised = failure
        .raised()
        .expect("the expired deadline has a projection");
    assert_eq!(raised.class, class);
    assert_eq!(raised.status, status);
    assert_eq!(raised.kind, kind);
    assert_eq!(raised.param, WireField::Null);
    assert_eq!(raised.code, None);
    // the shared mapping prefixes the handler's own line, which LiteLLM raises unwrapped
    assert!(raised.message.starts_with(prefix));
    // the handler fails before its client is asked for anything
    assert_eq!(spy.call_count(), 0);
}

// REQUEST_TIMEOUT is the same effective timeout, so a negative constant takes the same handler path
#[tokio::test]
async fn a_negative_request_timeout_expires_before_the_send() {
    let spy = Spy::replying(success());
    let fixture = Fixture::new();
    let failure = call(Which::Update, &spy, &fixture, -1.0_f64)
        .await
        .expect_err("the deadline expired");
    let raised = failure
        .raised()
        .expect("the expired deadline has a projection");
    assert_eq!(raised.class, "Timeout");
    assert_eq!(raised.status, 408_u16);
    assert_eq!(spy.call_count(), 0);
}

#[tokio::test]
async fn update_logs_the_ids_and_the_attributes() {
    let spy = Spy::replying(success());
    let mut fixture = Fixture::new();
    fixture.arguments.attributes = Some(attributes());
    let seen = recorded(Which::Update, &spy, &fixture, REQUEST_TIMEOUT).await;
    assert_eq!(seen.model, "");
    assert_eq!(seen.provider_name.as_deref(), Some("openai"));
    // the log keeps the argument as the caller gave it, before the wire cleaning drops non-strings
    assert_eq!(
        seen.optional,
        IndexMap::from([
            (
                String::from("vector_store_id"),
                Json::Text(String::from(VECTOR_STORE_ID)),
            ),
            (String::from("file_id"), Json::Text(String::from(FILE_ID)),),
            (String::from("attributes"), attributes_json()),
        ])
    );
    assert_eq!(
        seen.litellm_params.get("vector_store_id"),
        Some(&Json::Text(String::from(VECTOR_STORE_ID)))
    );
    assert_eq!(
        seen.litellm_params.get("litellm_call_id"),
        Some(&Json::Null)
    );
    assert_eq!(
        seen.details.get("custom_llm_provider"),
        Some(&Json::Text(String::from("openai")))
    );
}

#[tokio::test]
async fn async_logging_keeps_the_call_id() {
    let spy = Spy::replying(success());
    let mut fixture = Fixture::new();
    fixture.arguments.attributes = Some(attributes());
    fixture.arguments.kwargs.insert(
        String::from("litellm_call_id"),
        Json::Text(String::from("call-a")),
    );
    let seen = recorded(Which::Aupdate, &spy, &fixture, REQUEST_TIMEOUT).await;
    assert_eq!(seen.operation, Operation::Update);
    assert_eq!(seen.mode, ClientKind::Asynchronous);
    assert_eq!(seen.model, "");
    assert_eq!(seen.optional.get("attributes"), Some(&attributes_json()));
    assert_eq!(
        seen.litellm_params.get("litellm_call_id"),
        Some(&Json::Text(String::from("call-a")))
    );
    // LiteLLM pops the flag before logging, so it never reaches litellm_params
    assert_eq!(seen.litellm_params.get("aupdate"), None);
}

#[tokio::test]
async fn adelete_sets_the_flag_itself() {
    let spy = Spy::replying(deleted());
    let fixture = Fixture::new();
    let seen = recorded(Which::Adelete, &spy, &fixture, REQUEST_TIMEOUT).await;
    assert_eq!(seen.operation, Operation::Delete);
    assert_eq!(seen.mode, ClientKind::Asynchronous);
    assert_eq!(seen.litellm_params.get("adelete"), None);
    assert_eq!(
        seen.litellm_params.get("litellm_call_id"),
        Some(&Json::Null)
    );
}

#[tokio::test]
async fn integer_flags_do_not_select_async() {
    let spy = Spy::replying(success());
    let mut update_fixture = Fixture::new();
    update_fixture
        .arguments
        .kwargs
        .insert(String::from("aupdate"), Json::Number(Number::from(1_u32)));
    let update_seen = recorded(Which::Update, &spy, &update_fixture, REQUEST_TIMEOUT).await;
    assert_eq!(update_seen.mode, ClientKind::Sync);
    // the pop takes the flag whatever its value, as LiteLLM's kwargs.pop does
    assert_eq!(update_seen.litellm_params.get("aupdate"), None);
    let mut delete_fixture = Fixture::new();
    delete_fixture
        .arguments
        .kwargs
        .insert(String::from("adelete"), Json::Number(Number::from(1_u32)));
    let delete_seen = recorded(Which::Delete, &spy, &delete_fixture, REQUEST_TIMEOUT).await;
    assert_eq!(delete_seen.mode, ClientKind::Sync);
    assert_eq!(delete_seen.litellm_params.get("adelete"), None);
}
