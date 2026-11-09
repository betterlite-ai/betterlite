use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;

use super::fake::ALLOW_SOURCE;
use super::fake::FakeHost;
use super::fake::LoadReply;
use super::fake::build;
use super::fake::inputs;
use super::fake::map;
use super::fake::object;
use super::fake::params;
use super::fake::text;
use super::fake::value;
use crate::custom_code::error::CustomCodeApplyError;
use crate::custom_code::inputs::CustomCodeInputs;
use crate::generic_guardrail_contracts::GuardrailInputType;

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

async fn applied_flag(request_data: &mut BTreeMap<String, GuardrailValue>) -> CustomCodeInputs {
    let host = host_returning(flag_result());
    let guardrail = build(params(ALLOW_SOURCE), &host);
    guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            request_data,
            GuardrailInputType::Request,
        )
        .await
        .unwrap()
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
    let Some(GuardrailValue::Object(bucket)) = request_data.get("metadata") else {
        panic!("expected the metadata bucket");
    };
    let Some(GuardrailValue::Array(records)) = bucket.get("standard_logging_guardrail_information")
    else {
        panic!("expected the flag record");
    };
    assert_eq!(records.len(), 1);
    let GuardrailValue::Object(record) = &records[0] else {
        panic!("expected the record to be an object");
    };
    assert_eq!(record.get("guardrail_name"), Some(&text("custom_code")));
    assert_eq!(
        record.get("guardrail_status"),
        Some(&text("guardrail_flagged"))
    );
    assert_eq!(
        record.get("start_time"),
        Some(&GuardrailValue::Decimal(0.005_f64))
    );
    assert_eq!(
        record.get("duration"),
        Some(&GuardrailValue::Decimal(0.0_f64))
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

#[tokio::test]
async fn an_unknown_action_warns_and_allows() {
    let result = object(vec![("action", text("explode"))]);
    let mut request_data = BTreeMap::new();
    run(result, &mut request_data, GuardrailInputType::Request)
        .await
        .unwrap();
    assert!(request_data.is_empty());
}

#[tokio::test]
async fn a_result_that_is_not_an_object_allows() {
    let mut request_data = BTreeMap::new();
    run(
        GuardrailValue::Text("not a dict".to_owned()),
        &mut request_data,
        GuardrailInputType::Request,
    )
    .await
    .unwrap();
    assert!(request_data.is_empty());
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
