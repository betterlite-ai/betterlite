use std::collections::BTreeMap;
use std::io;
use std::io::Write;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use tokio::runtime::Builder;
use tracing::Level;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt;

use super::fake::ALLOW_SOURCE;
use super::fake::ASYNC_SOURCE;
use super::fake::FakeHost;
use super::fake::LoadReply;
use super::fake::build;
use super::fake::build_with_clock;
use super::fake::fixed_clock;
use super::fake::inputs;
use super::fake::map;
use super::fake::object;
use super::fake::params;
use super::fake::text;
use super::fake::ticking_clock;
use super::fake::value;
use crate::custom_code::error::CustomCodeApplyError;
use crate::custom_code::inputs::CustomCodeInputs;
use crate::generic_guardrail_contracts::GuardrailInputType;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture<T, F>(level: Level, run: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = fmt()
        .with_max_level(level)
        .with_ansi(false)
        .with_target(false)
        .with_writer(move || writer.clone())
        .finish();
    let result = with_default(subscriber, run);
    let output = String::from_utf8(captured.0.lock().expect("poisoned").clone())
        .expect("tracing output is utf8");
    (result, output)
}

fn host_returning(result: GuardrailValue) -> Arc<FakeHost> {
    FakeHost::new(LoadReply::Module(value(result)))
}

async fn run(
    result: GuardrailValue,
    request_data: &mut BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
) -> Result<(), CustomCodeApplyError> {
    let host = host_returning(result);
    let guardrail = build(params(ALLOW_SOURCE), &host);
    guardrail
        .apply_guardrail(inputs(&["hello"]), request_data, input_type)
        .await
        .map(|_returned| ())
}

fn apply_with_logs(
    result: GuardrailValue,
    input_type: GuardrailInputType,
) -> (Result<CustomCodeInputs, CustomCodeApplyError>, String) {
    let host = host_returning(result);
    let guardrail = build(params(ALLOW_SOURCE), &host);
    let runtime = Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("the test runtime builds");
    capture(Level::WARN, || {
        let mut request_data = BTreeMap::new();
        runtime.block_on(guardrail.apply_guardrail(
            inputs(&["hello"]),
            &mut request_data,
            input_type,
        ))
    })
}

fn records_of(request_data: &BTreeMap<String, GuardrailValue>) -> Vec<GuardrailValue> {
    let Some(GuardrailValue::Object(bucket)) = request_data.get("metadata") else {
        panic!("expected the metadata bucket");
    };
    let Some(GuardrailValue::Array(records)) = bucket.get("standard_logging_guardrail_information")
    else {
        panic!("expected the flag records");
    };
    records.clone()
}

fn record_fields(record: &GuardrailValue) -> &BTreeMap<String, GuardrailValue> {
    let GuardrailValue::Object(fields) = record else {
        panic!("expected the record to be an object");
    };
    fields
}

fn decimal(fields: &BTreeMap<String, GuardrailValue>, key: &str) -> f64 {
    let Some(GuardrailValue::Decimal(value)) = fields.get(key) else {
        panic!("expected a decimal {key}");
    };
    *value
}

#[tokio::test]
async fn block_on_a_response_is_a_400_with_litellms_detail() {
    let result = object(vec![
        ("action", text("block")),
        ("reason", text("nope")),
        (
            "detection_info",
            object(vec![("matched_phrase", text("nope"))]),
        ),
    ]);
    let mut request_data = BTreeMap::new();
    let error = run(result, &mut request_data, GuardrailInputType::Response)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), Some(StatusCode::BAD_REQUEST));
    let detail = error.block_detail().unwrap();
    assert_eq!(detail.error, "nope");
    assert_eq!(detail.guardrail, "custom_code");
    assert_eq!(
        detail.detection_info.get("matched_phrase"),
        Some(&text("nope"))
    );
}

#[tokio::test]
async fn block_without_a_reason_uses_litellms_default_text() {
    let result = object(vec![("action", text("block"))]);
    let mut request_data = BTreeMap::new();
    let error = run(result, &mut request_data, GuardrailInputType::Response)
        .await
        .unwrap_err();
    let detail = error.block_detail().unwrap();
    assert_eq!(detail.error, "Blocked by custom code guardrail");
    assert!(detail.detection_info.is_empty());
}

#[tokio::test]
async fn block_on_a_request_is_a_passthrough_not_an_http_error() {
    let result = object(vec![
        ("action", text("block")),
        ("reason", text("nope")),
        ("detection_info", object(vec![("k", text("v"))])),
    ]);
    let mut request_data = BTreeMap::new();
    let error = run(result, &mut request_data, GuardrailInputType::Request)
        .await
        .unwrap_err();
    assert!(error.status_code().is_none());
    assert!(error.block_detail().is_none());
    let CustomCodeApplyError::Passthrough {
        message,
        detection_info,
    } = error
    else {
        panic!("expected a passthrough");
    };
    assert_eq!(message, "nope");
    assert_eq!(detection_info.get("k"), Some(&text("v")));
}

fn flag_result() -> GuardrailValue {
    object(vec![
        ("action", text("flag")),
        ("reason", text("soft")),
        ("metadata", object(vec![("k", text("v"))])),
    ])
}

async fn applied_flag_with_clock(
    result: GuardrailValue,
    source: &str,
    request_data: &mut BTreeMap<String, GuardrailValue>,
    clock: fn() -> Duration,
) -> CustomCodeInputs {
    let host = host_returning(result);
    let guardrail = build_with_clock(params(source), &host, clock);
    guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            request_data,
            GuardrailInputType::Request,
        )
        .await
        .unwrap()
}

async fn applied_flag(request_data: &mut BTreeMap<String, GuardrailValue>) -> CustomCodeInputs {
    applied_flag_with_clock(flag_result(), ALLOW_SOURCE, request_data, fixed_clock).await
}

#[tokio::test]
async fn flag_leaves_the_inputs_alone() {
    let mut request_data = map(vec![("metadata", object(vec![("keep", text("yes"))]))]);
    let returned = applied_flag(&mut request_data).await;
    assert_eq!(returned, inputs(&["hello"]));
    let Some(GuardrailValue::Object(bucket)) = request_data.get("metadata") else {
        panic!("expected the metadata bucket");
    };
    assert_eq!(bucket.get("keep"), Some(&text("yes")));
    assert!(bucket.contains_key("standard_logging_guardrail_information"));
}

#[tokio::test]
async fn flag_records_a_guardrail_flagged_entry() {
    let mut request_data = map(vec![("metadata", object(vec![]))]);
    let _returned = applied_flag(&mut request_data).await;
    let records = records_of(&request_data);
    assert_eq!(records.len(), 1);
    let record = record_fields(&records[0]);
    assert_eq!(record.get("guardrail_name"), Some(&text("custom_code")));
    assert_eq!(
        record.get("guardrail_status"),
        Some(&text("guardrail_flagged"))
    );
    let Some(GuardrailValue::Object(response)) = record.get("guardrail_response") else {
        panic!("expected the guardrail response");
    };
    assert_eq!(response.get("action"), Some(&text("flag")));
    assert_eq!(response.get("reason"), Some(&text("soft")));
    assert_eq!(response.get("input_type"), Some(&text("request")));
    assert_eq!(
        response.get("metadata"),
        Some(&object(vec![("k", text("v"))]))
    );
}

#[tokio::test]
async fn flag_records_the_clock_times() {
    let mut request_data = map(vec![("metadata", object(vec![]))]);
    let _returned = applied_flag_with_clock(
        flag_result(),
        ASYNC_SOURCE,
        &mut request_data,
        ticking_clock,
    )
    .await;
    let records = records_of(&request_data);
    let fields = record_fields(&records[0]);
    let start = decimal(fields, "start_time");
    let end = decimal(fields, "end_time");
    let duration = decimal(fields, "duration");
    assert_eq!(
        start.to_bits(),
        Duration::from_secs(1).as_secs_f64().to_bits(),
        "start_time must be the clock read taken at the top of apply_guardrail"
    );
    assert_eq!(
        end.to_bits(),
        Duration::from_millis(1010).as_secs_f64().to_bits()
    );
    assert_eq!(
        duration.to_bits(),
        Duration::from_millis(10).as_secs_f64().to_bits(),
        "duration must be end_time minus start_time"
    );
}

#[tokio::test]
async fn flag_without_a_reason_uses_litellms_default_text() {
    let result = object(vec![("action", text("flag"))]);
    let mut request_data = map(vec![("metadata", object(vec![]))]);
    let _returned =
        applied_flag_with_clock(result, ALLOW_SOURCE, &mut request_data, fixed_clock).await;
    let records = records_of(&request_data);
    let record = record_fields(&records[0]);
    let Some(GuardrailValue::Object(response)) = record.get("guardrail_response") else {
        panic!("expected the guardrail response");
    };
    assert_eq!(
        response.get("reason"),
        Some(&text("Flagged by custom code guardrail"))
    );
}

#[tokio::test]
async fn flag_appends_to_an_existing_record_list() {
    let earlier = object(vec![("guardrail_name", text("earlier"))]);
    let mut request_data = map(vec![(
        "metadata",
        object(vec![(
            "standard_logging_guardrail_information",
            GuardrailValue::Array(vec![earlier.clone()]),
        )]),
    )]);
    let _returned = applied_flag(&mut request_data).await;
    let records = records_of(&request_data);
    assert_eq!(records.len(), 2);
    assert_eq!(records.first(), Some(&earlier));
    assert_eq!(
        record_fields(&records[1]).get("guardrail_name"),
        Some(&text("custom_code"))
    );
}

#[tokio::test]
async fn flag_wraps_a_non_list_value_before_appending() {
    let earlier = text("earlier");
    let mut request_data = map(vec![(
        "metadata",
        object(vec![(
            "standard_logging_guardrail_information",
            earlier.clone(),
        )]),
    )]);
    let _returned = applied_flag(&mut request_data).await;
    let records = records_of(&request_data);
    assert_eq!(records.len(), 2);
    assert_eq!(records.first(), Some(&earlier));
    assert_eq!(
        record_fields(&records[1]).get("guardrail_name"),
        Some(&text("custom_code"))
    );
}

#[tokio::test]
async fn flag_uses_the_litellm_metadata_bucket_when_the_request_has_one() {
    let result = object(vec![("action", text("flag")), ("reason", text("soft"))]);
    let mut request_data = map(vec![
        ("metadata", object(vec![("keep", text("meta"))])),
        ("litellm_metadata", object(vec![("keep", text("litellm"))])),
    ]);
    run(result, &mut request_data, GuardrailInputType::Request)
        .await
        .unwrap();
    let Some(GuardrailValue::Object(plain)) = request_data.get("metadata") else {
        panic!("expected the metadata bucket");
    };
    assert!(!plain.contains_key("standard_logging_guardrail_information"));
    let Some(GuardrailValue::Object(bucket)) = request_data.get("litellm_metadata") else {
        panic!("expected the litellm metadata bucket");
    };
    assert!(bucket.contains_key("standard_logging_guardrail_information"));
    assert_eq!(bucket.get("keep"), Some(&text("litellm")));
}

#[tokio::test]
async fn flag_replaces_a_metadata_bucket_that_is_not_an_object() {
    let result = object(vec![("action", text("flag"))]);
    let mut request_data = map(vec![("metadata", text("not a bucket"))]);
    run(result, &mut request_data, GuardrailInputType::Request)
        .await
        .unwrap();
    let Some(GuardrailValue::Object(bucket)) = request_data.get("metadata") else {
        panic!("expected the replaced metadata bucket");
    };
    assert!(bucket.contains_key("standard_logging_guardrail_information"));
}

#[test]
fn an_unknown_action_warns_and_allows() {
    let (returned, logs) = apply_with_logs(
        object(vec![("action", text("explode"))]),
        GuardrailInputType::Request,
    );
    assert_eq!(returned.unwrap(), inputs(&["hello"]));
    assert!(logs.contains(
        "Custom code guardrail 'custom_code': Unknown action 'explode'. Treating as allow."
    ));
}

#[test]
fn a_result_that_is_not_an_object_warns_and_allows() {
    let (returned, logs) = apply_with_logs(
        GuardrailValue::Text("not a dict".to_owned()),
        GuardrailInputType::Request,
    );
    assert_eq!(returned.unwrap(), inputs(&["hello"]));
    assert!(logs.contains(
        "Custom code guardrail 'custom_code': Expected dict result, got str. Treating as allow."
    ));
}

#[tokio::test]
async fn a_block_reason_that_is_not_text_names_the_field() {
    let result = object(vec![
        ("action", text("block")),
        ("reason", GuardrailValue::SignedInteger(5)),
    ]);
    let mut request_data = BTreeMap::new();
    let error = run(result, &mut request_data, GuardrailInputType::Response)
        .await
        .unwrap_err();
    let CustomCodeApplyError::Execution(failure) = error else {
        panic!("expected an execution error");
    };
    assert!(failure.to_string().contains("'reason'"));
}

#[tokio::test]
async fn a_flag_metadata_that_is_not_an_object_names_the_field() {
    let result = object(vec![
        ("action", text("flag")),
        ("metadata", GuardrailValue::SignedInteger(5)),
    ]);
    let mut request_data = BTreeMap::new();
    let error = run(result, &mut request_data, GuardrailInputType::Request)
        .await
        .unwrap_err();
    let CustomCodeApplyError::Execution(failure) = error else {
        panic!("expected an execution error");
    };
    assert!(failure.to_string().contains("'metadata'"));
}

#[tokio::test]
async fn modify_texts_that_are_not_a_list_of_texts_names_the_field() {
    let result = object(vec![
        ("action", text("modify")),
        ("texts", text("not a list")),
    ]);
    let mut request_data = BTreeMap::new();
    let error = run(result, &mut request_data, GuardrailInputType::Request)
        .await
        .unwrap_err();
    let CustomCodeApplyError::Execution(failure) = error else {
        panic!("expected an execution error");
    };
    assert!(failure.to_string().contains("'texts'"));
}

#[tokio::test]
async fn modify_images_with_a_non_text_item_names_the_field() {
    let result = object(vec![
        ("action", text("modify")),
        (
            "images",
            GuardrailValue::Array(vec![GuardrailValue::SignedInteger(5)]),
        ),
    ]);
    let mut request_data = BTreeMap::new();
    let error = run(result, &mut request_data, GuardrailInputType::Request)
        .await
        .unwrap_err();
    let CustomCodeApplyError::Execution(failure) = error else {
        panic!("expected an execution error");
    };
    assert!(failure.to_string().contains("'images'"));
}

#[tokio::test]
async fn modify_tool_calls_that_are_not_a_list_names_the_field() {
    let result = object(vec![
        ("action", text("modify")),
        ("tool_calls", text("not a list")),
    ]);
    let mut request_data = BTreeMap::new();
    let error = run(result, &mut request_data, GuardrailInputType::Request)
        .await
        .unwrap_err();
    let CustomCodeApplyError::Execution(failure) = error else {
        panic!("expected an execution error");
    };
    assert!(failure.to_string().contains("'tool_calls'"));
}

#[tokio::test]
async fn detection_info_that_is_not_an_object_names_the_field() {
    let result = object(vec![
        ("action", text("block")),
        ("detection_info", GuardrailValue::SignedInteger(5)),
    ]);
    let mut request_data = BTreeMap::new();
    let error = run(result, &mut request_data, GuardrailInputType::Response)
        .await
        .unwrap_err();
    let CustomCodeApplyError::Execution(failure) = error else {
        panic!("expected an execution error");
    };
    assert!(failure.to_string().contains("'detection_info'"));
}
