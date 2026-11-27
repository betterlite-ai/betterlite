#![cfg(test)]

use std::time::Duration;

use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_models::decisions::Json;
use betterlite_openai::evals::types::{EvalOrder, EvalOrderBy};
use indexmap::IndexMap;
use pyo3::PyErr;
use pyo3::Python;
use rstest::rstest;
use serde::Deserialize;

use super::{ConvertError, Fields, parse, timeout};
use crate::eval_calls::error::{EvalOperationFailure, payload};
use crate::eval_calls::payload::{CreatePayload, IdPayload, ListPayload, UpdatePayload};

const CREATE: &str = r#"{
    "data_source_config": {"type": "custom", "item_schema": {"type": "object"}},
    "testing_criteria": [{"type": "custom", "function_id": "fn-1"}],
    "name": "probe",
    "metadata": {"tag": "x"},
    "extra_body": {"note": "n"},
    "extra_headers": {"x-test": "1"},
    "extra_query": {"limit": 9},
    "timeout": 2.5,
    "custom_llm_provider": "openai",
    "api_base": "http://local/v1",
    "api_key": "sk-test",
    "max_retries": 2
}"#;

const LIST: &str = r#"{
    "limit": 5,
    "after": "eval_a",
    "before": "eval_b",
    "order": "asc",
    "order_by": "updated_at",
    "extra_query": {"limit": 9, "after": "eval_c", "unknown": "x"},
    "api_base": "http://local"
}"#;

#[test]
fn request_create_reads_the_typed_fields() {
    let payload = CreatePayload::parse(CREATE).unwrap();
    assert_eq!(payload.name.as_deref(), Some("probe"));
    assert_eq!(
        serde_json::to_string(&payload.data_source_config).unwrap(),
        r#"{"type":"custom","item_schema":{"type":"object"}}"#
    );
    assert_eq!(
        serde_json::to_string(&payload.testing_criteria).unwrap(),
        r#"[{"type":"custom","function_id":"fn-1"}]"#
    );
    assert_eq!(
        payload.metadata.unwrap().get("tag"),
        Some(&Json::Text("x".to_owned()))
    );
    assert_eq!(
        payload.extra_body.get("note"),
        Some(&Json::Text("n".to_owned()))
    );
}

#[test]
fn request_create_splits_the_shared_parameters_from_the_keywords() {
    let payload = CreatePayload::parse(CREATE).unwrap();
    assert_eq!(
        payload
            .params
            .extra_headers
            .as_ref()
            .unwrap()
            .get("x-test")
            .map(String::as_str),
        Some("1")
    );
    assert!(matches!(
        payload.params.extra_query.get("limit"),
        Some(Json::Number(value)) if value.as_i64() == Some(9)
    ));
    assert_eq!(
        payload.params.timeout,
        Some(Duration::try_from_secs_f64(2.5).unwrap())
    );
    assert_eq!(
        payload.params.custom_llm_provider.as_deref(),
        Some("openai")
    );
    assert_eq!(
        payload.params.kwargs.get("api_base"),
        Some(&Json::Text("http://local/v1".to_owned()))
    );
    assert_eq!(
        payload.params.kwargs.get("api_key"),
        Some(&Json::Text("sk-test".to_owned()))
    );
    assert!(matches!(
        payload.params.kwargs.get("max_retries"),
        Some(Json::Number(value)) if value.as_i64() == Some(2)
    ));
}

#[rstest]
#[case::metadata("metadata")]
#[case::extra_body("extra_body")]
#[case::extra_headers("extra_headers")]
#[case::extra_query("extra_query")]
#[case::timeout("timeout")]
fn request_create_named_parameters_leave_the_keywords(#[case] named: &str) {
    let payload = CreatePayload::parse(CREATE).unwrap();
    assert!(
        !payload.params.kwargs.contains_key(named),
        "{named} stayed in kwargs"
    );
}

#[test]
fn request_create_requires_both_typed_fields() {
    let missing_config = CreatePayload::parse(r#"{"testing_criteria": []}"#);
    assert!(matches!(
        missing_config,
        Err(ConvertError::FieldType {
            name: "data_source_config",
            ..
        })
    ));
    let missing_criteria =
        CreatePayload::parse(r#"{"data_source_config": {"type": "custom", "item_schema": {}}}"#);
    assert!(matches!(
        missing_criteria,
        Err(ConvertError::FieldType {
            name: "testing_criteria",
            ..
        })
    ));
    let nulled = CreatePayload::parse(r#"{"data_source_config": null, "testing_criteria": []}"#);
    assert!(matches!(
        nulled,
        Err(ConvertError::FieldType {
            name: "data_source_config",
            ..
        })
    ));
}

#[test]
fn request_create_reads_a_null_name_and_metadata_as_absent() {
    let payload = CreatePayload::parse(
        r#"{"data_source_config": {"type": "logs"}, "testing_criteria": [], "name": null, "metadata": null}"#,
    )
    .unwrap();
    assert_eq!(payload.name, None);
    assert!(payload.metadata.is_none());
}

#[test]
fn request_list_reads_every_pagination_field() {
    let payload = ListPayload::parse(LIST).unwrap();
    assert_eq!(payload.limit, Some(5));
    assert_eq!(payload.after.as_deref(), Some("eval_a"));
    assert_eq!(payload.before.as_deref(), Some("eval_b"));
    assert_eq!(payload.order, Some(EvalOrder::Asc));
    assert_eq!(payload.order_by, Some(EvalOrderBy::UpdatedAt));
    assert_eq!(
        payload.params.kwargs.get("api_base"),
        Some(&Json::Text("http://local".to_owned()))
    );
}

#[test]
fn request_list_keeps_extra_query_as_given() {
    let payload = ListPayload::parse(LIST).unwrap();
    assert_eq!(
        payload.params.extra_query.get("after"),
        Some(&Json::Text("eval_c".to_owned()))
    );
    assert_eq!(
        payload.params.extra_query.get("unknown"),
        Some(&Json::Text("x".to_owned()))
    );
}

#[test]
fn request_list_reads_nulls_as_absent() {
    let payload = ListPayload::parse(
        r#"{"limit": null, "after": null, "before": null, "order": null, "order_by": null}"#,
    )
    .unwrap();
    assert_eq!(payload.limit, None);
    assert_eq!(payload.after, None);
    assert_eq!(payload.before, None);
    assert_eq!(payload.order, None);
    assert_eq!(payload.order_by, None);
}

#[test]
fn request_update_reads_the_typed_fields() {
    let payload = UpdatePayload::parse(
        r#"{"eval_id": "eval_1", "name": "old", "metadata": {"team": "a"}, "extra_body": {"note": "n"}}"#,
    )
    .unwrap();
    assert_eq!(payload.eval_id, "eval_1");
    assert_eq!(payload.name.as_deref(), Some("old"));
    assert_eq!(
        payload.metadata.unwrap().get("team"),
        Some(&Json::Text("a".to_owned()))
    );
    assert_eq!(
        payload.extra_body.get("note"),
        Some(&Json::Text("n".to_owned()))
    );
}

#[test]
fn request_id_reads_the_eval_id_and_keeps_extra_query() {
    let payload = IdPayload::parse(r#"{"eval_id": "eval_1", "extra_query": {"a": "b"}}"#).unwrap();
    assert_eq!(payload.eval_id, "eval_1");
    assert_eq!(
        payload.params.extra_query.get("a"),
        Some(&Json::Text("b".to_owned()))
    );
    assert!(payload.params.kwargs.is_empty());
}

#[test]
fn request_the_provider_defaults_to_openai() {
    let payload = IdPayload::parse(r#"{"eval_id": "eval_1"}"#).unwrap();
    assert_eq!(payload.params.provider(), "openai");
    let named =
        IdPayload::parse(r#"{"eval_id": "eval_1", "custom_llm_provider": "openai"}"#).unwrap();
    assert_eq!(named.params.provider(), "openai");
    let other =
        IdPayload::parse(r#"{"eval_id": "eval_1", "custom_llm_provider": "anthropic"}"#).unwrap();
    assert_eq!(other.params.provider(), "anthropic");
}

#[test]
fn request_unknown_keywords_stay_kwargs_even_when_null() {
    let payload =
        IdPayload::parse(r#"{"eval_id": "e", "max_retries": null, "organization": "org"}"#)
            .unwrap();
    assert_eq!(payload.params.kwargs.get("max_retries"), Some(&Json::Null));
    assert_eq!(
        payload.params.kwargs.get("organization"),
        Some(&Json::Text("org".to_owned()))
    );
}

#[test]
fn request_timeout_reads_seconds() {
    let mut fields = parse::<Fields>(r#"{"timeout": 2.5}"#).unwrap();
    assert_eq!(
        timeout(&mut fields, "openai").unwrap(),
        Some(Duration::try_from_secs_f64(2.5).unwrap())
    );
    let mut zero = parse::<Fields>(r#"{"timeout": 0}"#).unwrap();
    assert_eq!(timeout(&mut zero, "openai").unwrap(), Some(Duration::ZERO));
    let mut nulled = parse::<Fields>(r#"{"timeout": null}"#).unwrap();
    assert_eq!(timeout(&mut nulled, "openai").unwrap(), None);
    let mut text = parse::<Fields>(r#"{"timeout": "soon"}"#).unwrap();
    assert!(matches!(
        timeout(&mut text, "openai"),
        Err(ConvertError::FieldType {
            name: "timeout",
            ..
        })
    ));
}

#[test]
fn request_a_wrong_typed_field_names_it() {
    let error = ListPayload::parse(r#"{"limit": "many"}"#).unwrap_err();
    assert!(matches!(
        error,
        ConvertError::FieldType { name: "limit", .. }
    ));
    let order = ListPayload::parse(r#"{"order": "sideways"}"#).unwrap_err();
    assert!(matches!(
        order,
        ConvertError::FieldType { name: "order", .. }
    ));
    let eval_id = IdPayload::parse(r#"{"eval_id": 7}"#).unwrap_err();
    assert!(matches!(
        eval_id,
        ConvertError::FieldType {
            name: "eval_id",
            ..
        }
    ));
}

#[derive(Debug, Deserialize)]
struct ErrorWire {
    #[serde(rename = "type")]
    class: String,
    message: String,
    status_code: Option<u16>,
    llm_provider: String,
    model: Option<String>,
    body: Option<IndexMap<String, Json>>,
}

fn context(model: Option<String>, body_code: Option<String>) -> ErrorContext {
    ErrorContext {
        message: "the cursor is not an eval id".to_owned(),
        model,
        provider: Some("openai".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: Some("invalid_request_error".to_owned()),
        body_param: Some("after".to_owned()),
        body_code,
    }
}

fn bad_request(model: Option<String>, body_code: Option<String>) -> ErrorClass {
    let details = ResponseDetails::with_status(context(model, body_code), Some(400));
    ErrorClass::BadRequest(Box::new(details))
}

fn wire(class: &ErrorClass, provider: &str) -> ErrorWire {
    let text = serde_json::to_string(&payload(class, provider)).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn error_payload_carries_the_class_status_and_body() {
    let error = wire(
        &bad_request(None, Some("invalid_cursor".to_owned())),
        "openai",
    );
    assert_eq!(error.class, "BadRequestError");
    assert_eq!(error.message, "the cursor is not an eval id");
    assert_eq!(error.status_code, Some(400));
    assert_eq!(error.llm_provider, "openai");
    assert_eq!(error.model, None);
    let body = error.body.unwrap();
    assert_eq!(
        body.get("type"),
        Some(&Json::Text("invalid_request_error".to_owned()))
    );
    assert_eq!(body.get("param"), Some(&Json::Text("after".to_owned())));
    assert_eq!(
        body.get("code"),
        Some(&Json::Text("invalid_cursor".to_owned()))
    );
}

#[test]
fn error_payload_omits_an_empty_model_and_body() {
    let empty_model = bad_request(Some(String::new()), None);
    let text = serde_json::to_string(&payload(&empty_model, "openai")).unwrap();
    let fields: IndexMap<String, Json> = serde_json::from_str(&text).unwrap();
    assert!(!fields.contains_key("model"));

    let bare = ErrorContext {
        message: "boom".to_owned(),
        model: None,
        provider: Some("openai".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    let unclassified = ErrorClass::Unclassified(Box::new(bare));
    let text = serde_json::to_string(&payload(&unclassified, "openai")).unwrap();
    let fields: IndexMap<String, Json> = serde_json::from_str(&text).unwrap();
    assert!(!fields.contains_key("body"));
}

#[test]
fn error_payload_leaves_out_a_missing_code() {
    let error = wire(&bad_request(None, None), "openai");
    assert_eq!(error.status_code, Some(400));
    let body = error.body.unwrap();
    assert_eq!(body.get("type"), Some(&Json::Null));
    assert_eq!(body.get("param"), Some(&Json::Null));
    assert!(!body.contains_key("code"));
}

#[test]
fn error_payload_keeps_the_provider_it_was_given() {
    let error = wire(&bad_request(None, None), "anthropic");
    assert_eq!(error.llm_provider, "anthropic");
}

// docs/compatibility.md answers a request value outside its declared type with a 400 naming the field
#[test]
fn error_a_wrong_typed_field_is_a_bad_request() {
    Python::initialize();
    let error = ListPayload::parse(r#"{"limit": "many"}"#).unwrap_err();
    let raised = PyErr::from(EvalOperationFailure::from(error));
    Python::attach(|py| {
        let message = raised.value(py).to_string();
        assert!(message.contains(r#""type":"BadRequestError""#), "{message}");
        assert!(message.contains(r#""status_code":400"#), "{message}");
        assert!(message.contains(r#""llm_provider":"openai""#), "{message}");
        assert!(message.contains(r#""param":"limit""#), "{message}");
        assert!(
            message.contains("the limit field does not have the declared type"),
            "{message}"
        );
    });
}
