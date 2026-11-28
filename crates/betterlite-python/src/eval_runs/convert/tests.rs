#![cfg(test)]

use std::time::Duration;

use betterlite_core::api::eval_run_create_list::error::{EvalRunCallError, EvalRunFailure};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use pyo3::PyErr;
use pyo3::Python;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use rstest::rstest;
use serde::Deserialize;

use super::{ConvertError, CreateRunPayload, Fields, ListRunPayload, RunIdPayload, parse, timeout};
use crate::eval_runs::error::{EvalOperationFailure, SessionError, native_error};

const CREATE: &str = r#"{
    "eval_id": "eval_1",
    "data_source": {"type": "inline", "samples": [{"item": 1}]},
    "name": "probe",
    "metadata": {"tag": "x"},
    "extra_body": {"metadata": {"sent": true}},
    "extra_headers": {"x-test": "1"},
    "extra_query": {"limit": 9},
    "timeout": 2.5,
    "custom_llm_provider": "openai",
    "api_base": "http://local/v1",
    "api_key": "sk-test",
    "max_retries": 2
}"#;

const LIST: &str = r#"{
    "eval_id": "eval_1",
    "limit": 5,
    "after": "run_a",
    "before": "run_b",
    "order": "asc",
    "extra_query": {"limit": 9, "after": "run_c", "unknown": "x"},
    "api_base": "http://local"
}"#;

#[test]
fn request_create_reads_the_typed_fields() {
    let payload = CreateRunPayload::parse(CREATE).unwrap();
    assert_eq!(payload.eval_id, "eval_1");
    assert_eq!(
        serde_json::to_string(&payload.data_source).unwrap(),
        r#"{"type":"inline","samples":[{"item":1}]}"#
    );
    assert_eq!(payload.name.as_deref(), Some("probe"));
    assert_eq!(
        serde_json::to_string(&payload.metadata).unwrap(),
        r#"{"tag":"x"}"#
    );
    assert_eq!(
        serde_json::to_string(&payload.extra_body).unwrap(),
        r#"{"metadata":{"sent":true}}"#
    );
}

#[test]
fn request_create_splits_the_shared_parameters_from_the_keywords() {
    let payload = CreateRunPayload::parse(CREATE).unwrap();
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
    assert_eq!(
        serde_json::to_string(&payload.params.extra_query).unwrap(),
        r#"{"limit":9}"#
    );
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
#[case::eval_id("eval_id")]
#[case::data_source("data_source")]
#[case::name("name")]
#[case::metadata("metadata")]
#[case::extra_body("extra_body")]
#[case::extra_headers("extra_headers")]
#[case::extra_query("extra_query")]
#[case::timeout("timeout")]
fn request_create_named_parameters_leave_the_keywords(#[case] named: &str) {
    let payload = CreateRunPayload::parse(CREATE).unwrap();
    assert!(
        !payload.params.kwargs.contains_key(named),
        "{named} stayed in kwargs"
    );
}

#[test]
fn request_create_requires_the_id_and_the_data_source() {
    let missing_id = CreateRunPayload::parse(r#"{"data_source": {"type": "inline"}}"#);
    assert!(matches!(
        missing_id,
        Err(ConvertError::FieldType {
            name: "eval_id",
            ..
        })
    ));
    let missing_source = CreateRunPayload::parse(r#"{"eval_id": "eval_1"}"#);
    assert!(matches!(
        missing_source,
        Err(ConvertError::FieldType {
            name: "data_source",
            ..
        })
    ));
    let nulled = CreateRunPayload::parse(r#"{"eval_id": "eval_1", "data_source": null}"#);
    assert!(matches!(
        nulled,
        Err(ConvertError::FieldType {
            name: "data_source",
            ..
        })
    ));
}

#[test]
fn request_create_reads_a_null_name_and_metadata_as_absent() {
    let payload = CreateRunPayload::parse(
        r#"{"eval_id": "eval_1", "data_source": {"type": "inline"}, "name": null, "metadata": null}"#,
    )
    .unwrap();
    assert_eq!(payload.name, None);
    assert!(payload.metadata.is_none());
    assert!(payload.extra_body.is_none());
}

#[test]
fn request_list_reads_every_pagination_field() {
    let payload = ListRunPayload::parse(LIST).unwrap();
    assert_eq!(payload.eval_id, "eval_1");
    assert_eq!(payload.limit, Some(5));
    assert_eq!(payload.after.as_deref(), Some("run_a"));
    assert_eq!(payload.before.as_deref(), Some("run_b"));
    assert_eq!(payload.order.as_deref(), Some("asc"));
    assert_eq!(
        payload.params.kwargs.get("api_base"),
        Some(&Json::Text("http://local".to_owned()))
    );
}

#[test]
fn request_list_keeps_extra_query_as_given() {
    let payload = ListRunPayload::parse(LIST).unwrap();
    assert_eq!(
        serde_json::to_string(&payload.params.extra_query).unwrap(),
        r#"{"limit":9,"after":"run_c","unknown":"x"}"#
    );
}

#[test]
fn request_list_reads_nulls_as_absent() {
    let payload = ListRunPayload::parse(
        r#"{"eval_id": "eval_1", "limit": null, "after": null, "before": null, "order": null}"#,
    )
    .unwrap();
    assert_eq!(payload.limit, None);
    assert_eq!(payload.after, None);
    assert_eq!(payload.before, None);
    assert_eq!(payload.order, None);
}

#[test]
fn request_run_id_reads_both_ids() {
    let payload = RunIdPayload::parse(
        r#"{"eval_id": "eval_1", "run_id": "run_1", "extra_query": {"a": "b"}}"#,
    )
    .unwrap();
    assert_eq!(payload.eval_id, "eval_1");
    assert_eq!(payload.run_id, "run_1");
    assert_eq!(
        serde_json::to_string(&payload.params.extra_query).unwrap(),
        r#"{"a":"b"}"#
    );
    assert!(payload.params.kwargs.is_empty());
}

#[test]
fn request_run_id_requires_the_run_id() {
    let missing = RunIdPayload::parse(r#"{"eval_id": "eval_1"}"#);
    assert!(matches!(
        missing,
        Err(ConvertError::FieldType { name: "run_id", .. })
    ));
}

#[test]
fn request_the_provider_defaults_to_openai() {
    let payload = RunIdPayload::parse(r#"{"eval_id": "eval_1", "run_id": "run_1"}"#).unwrap();
    assert_eq!(payload.params.provider(), "openai");
    let other = RunIdPayload::parse(
        r#"{"eval_id": "eval_1", "run_id": "run_1", "custom_llm_provider": "anthropic"}"#,
    )
    .unwrap();
    assert_eq!(other.params.provider(), "anthropic");
}

#[test]
fn request_unknown_keywords_stay_kwargs_even_when_null() {
    let payload = RunIdPayload::parse(
        r#"{"eval_id": "e", "run_id": "r", "max_retries": null, "organization": "org"}"#,
    )
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

// docs/compatibility.md answers a timeout no Duration can hold with a 400 that names the field
#[rstest]
#[case::negative("-1.0")]
#[case::too_large("1e300")]
fn request_a_timeout_that_is_not_a_duration_is_a_bad_request(#[case] given: &str) {
    let error = ListRunPayload::parse(&format!(r#"{{"eval_id": "eval_1", "timeout": {given}}}"#))
        .err()
        .unwrap();
    assert!(matches!(
        error,
        ConvertError::FieldType {
            name: "timeout",
            ..
        }
    ));
}

// JSON has no non-finite numbers, so the parse refuses a NaN or infinite timeout
#[test]
fn request_a_non_finite_timeout_is_a_json_error() {
    let nan = ListRunPayload::parse(r#"{"eval_id": "eval_1", "timeout": NaN}"#);
    assert!(matches!(nan, Err(ConvertError::Json { .. })));
    let infinity = ListRunPayload::parse(r#"{"eval_id": "eval_1", "timeout": Infinity}"#);
    assert!(matches!(infinity, Err(ConvertError::Json { .. })));
}

#[test]
fn request_a_wrong_typed_field_names_it() {
    let limit = ListRunPayload::parse(r#"{"eval_id": "eval_1", "limit": "many"}"#)
        .err()
        .unwrap();
    assert!(matches!(
        limit,
        ConvertError::FieldType { name: "limit", .. }
    ));
    let order = ListRunPayload::parse(r#"{"eval_id": "eval_1", "order": 7}"#)
        .err()
        .unwrap();
    assert!(matches!(
        order,
        ConvertError::FieldType { name: "order", .. }
    ));
    let source = CreateRunPayload::parse(r#"{"eval_id": "eval_1", "data_source": "x"}"#)
        .err()
        .unwrap();
    assert!(matches!(
        source,
        ConvertError::FieldType {
            name: "data_source",
            ..
        }
    ));
    let run_id = RunIdPayload::parse(r#"{"eval_id": "eval_1", "run_id": 7}"#)
        .err()
        .unwrap();
    assert!(matches!(
        run_id,
        ConvertError::FieldType { name: "run_id", .. }
    ));
}

// docs/compatibility.md answers a request value outside its declared type with a 400 naming the field
#[test]
fn error_a_wrong_typed_field_is_a_bad_request() {
    Python::initialize();
    let error = ListRunPayload::parse(r#"{"eval_id": "eval_1", "limit": "many"}"#)
        .err()
        .unwrap();
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

#[test]
fn error_a_missing_required_field_is_a_bad_request() {
    Python::initialize();
    let error = RunIdPayload::parse(r#"{"eval_id": "eval_1"}"#)
        .err()
        .unwrap();
    let raised = PyErr::from(EvalOperationFailure::from(error));
    Python::attach(|py| {
        let message = raised.value(py).to_string();
        assert!(message.contains(r#""type":"BadRequestError""#), "{message}");
        assert!(message.contains(r#""param":"run_id""#), "{message}");
    });
}

#[test]
fn error_bad_json_is_a_value_error() {
    Python::initialize();
    let error = RunIdPayload::parse("not json").err().unwrap();
    let raised = PyErr::from(EvalOperationFailure::from(error));
    Python::attach(|py| assert!(raised.is_instance_of::<PyValueError>(py)));
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

fn bad_request(model: Option<String>, body_code: Option<String>) -> ErrorClass {
    let context = ErrorContext {
        message: "the cursor is not a run id".to_owned(),
        model,
        provider: Some("openai".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: Some("invalid_request_error".to_owned()),
        body_param: Some("after".to_owned()),
        body_code,
    };
    ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, Some(400))))
}

fn wire(class: &ErrorClass, provider: &str) -> ErrorWire {
    let raised = native_error(class, provider);
    Python::attach(|py| {
        let text = raised.value(py).to_string();
        serde_json::from_str(&text).unwrap()
    })
}

#[test]
fn error_payload_carries_the_class_status_and_body() {
    Python::initialize();
    let error = wire(
        &bad_request(None, Some("invalid_cursor".to_owned())),
        "openai",
    );
    assert_eq!(error.class, "BadRequestError");
    assert_eq!(error.message, "the cursor is not a run id");
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
    Python::initialize();
    let empty_model = bad_request(Some(String::new()), None);
    let raised = native_error(&empty_model, "openai");
    Python::attach(|py| {
        let text = raised.value(py).to_string();
        let fields: IndexMap<String, Json> = serde_json::from_str(&text).unwrap();
        assert!(!fields.contains_key("model"));
    });
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
    let raised = native_error(&ErrorClass::Unclassified(Box::new(bare)), "openai");
    Python::attach(|py| {
        let text = raised.value(py).to_string();
        let fields: IndexMap<String, Json> = serde_json::from_str(&text).unwrap();
        assert!(!fields.contains_key("body"));
    });
}

#[test]
fn error_payload_keeps_the_provider_it_was_given() {
    Python::initialize();
    let error = wire(&bad_request(None, None), "anthropic");
    assert_eq!(error.llm_provider, "anthropic");
    let body = error.body.unwrap();
    assert!(!body.contains_key("code"));
}

// exception_type maps an unsupported run provider to a 400, and the payload keeps that class and message
#[test]
fn error_a_core_failure_keeps_its_class() {
    Python::initialize();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let failure = EvalRunFailure::of(
        EvalRunCallError::UnsupportedCreate {
            provider: "anthropic".to_owned(),
        },
        "anthropic",
        &redactor,
        IndexMap::new(),
    );
    let answer: Result<(), EvalRunFailure> = Err(failure);
    let raised = match EvalOperationFailure::reply(answer, "anthropic") {
        Ok(_) => panic!("the failure has to surface"),
        Err(failure) => PyErr::from(failure),
    };
    Python::attach(|py| {
        let message = raised.value(py).to_string();
        assert!(message.contains(r#""type":"BadRequestError""#), "{message}");
        assert!(message.contains(r#""status_code":400"#), "{message}");
        assert!(
            message.contains(r#""llm_provider":"anthropic""#),
            "{message}"
        );
        assert!(
            message.contains("CREATE run is not supported for anthropic"),
            "{message}"
        );
    });
}

#[test]
fn error_a_session_failure_is_a_runtime_error() {
    Python::initialize();
    let source = serde_json::from_str::<Json>("{").unwrap_err();
    let raised = PyErr::from(SessionError::Encode { source });
    Python::attach(|py| assert!(raised.is_instance_of::<PyRuntimeError>(py)));
}
