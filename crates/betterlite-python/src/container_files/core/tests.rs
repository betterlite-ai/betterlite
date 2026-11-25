#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::env_integer::timeout::TimeoutError;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::openai_credentials::Raw;
use betterlite_core::api::container_file_endpoints::handler::{
    ContainerConfigLookup as _, ContainerIdDecoder as _,
};
use betterlite_core::api::response_id_codecs::container::build_container_id;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::container_contracts::records::{
    ContainerFileListResponse, ContainerFileObject, DeleteContainerFileResponse,
};
use betterlite_openai::containers::container_file_http::ContainerFileReply;
use betterlite_openai::containers::costs::ContainerCosts;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::openai_base_config::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_wrapper::ports::WrapperClock as _;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use pyo3_async_runtimes::tokio::get_runtime;
use rstest::rstest;
use rust_decimal::Decimal;
use tokio::time::sleep;
use uuid::Uuid;

use super::{Answer, SessionCore, answer, json_text};
use crate::completion;
use crate::container_files::convert::{CallWire, ConvertError, NativeFilePayload, parse};
use crate::container_files::error::{SessionError, SessionFailure, binary, native_error};
use crate::container_files::stack;
use crate::container_files::{NativeContainerFileSession, register};

const FILE_JSON: &str = concat!(
    r#"{"id":"cfile_1","object":"container.file","container_id":"cntr_1","bytes":12,"#,
    r#""created_at":1750000000,"path":"/mnt/data/a.txt","source":"user"}"#,
);

const LIST_JSON: &str = concat!(
    r#"{"object":"list","data":[{"id":"cfile_1","object":"container.file","#,
    r#""container_id":"cntr_1","bytes":12,"created_at":1750000000,"#,
    r#""path":"/mnt/data/a.txt","source":"user"}],"first_id":"cfile_1","#,
    r#""last_id":"cfile_1","has_more":false}"#,
);

const DELETED_JSON: &str = r#"{"id":"cfile_1","object":"container.file.deleted","deleted":true}"#;

const CONTENT_JSON: &str = r#"{"object":"file_content","text":"hello"}"#;

fn core() -> SessionCore {
    // an explicit empty environment keeps the test off the process's proxy and credential variables
    SessionCore::build(r#"{"environment":{}}"#).unwrap()
}

fn wire(text: &str) -> CallWire {
    parse(text).unwrap()
}

fn mocked(operation: &str, arguments: &str, body: &str) -> String {
    format!(
        r#"{{"operation":"{operation}","arguments":{{{arguments},"mock_response":{}}}}}"#,
        serde_json::to_string(body).unwrap()
    )
}

fn session(py: Python<'_>) -> NativeContainerFileSession {
    NativeContainerFileSession {
        core: Arc::new(core()),
        error_type: PyErr::new_type(py, c"probe.NativeContainerFileError", None, None, None)
            .unwrap(),
    }
}

#[test]
fn call_list_answers_the_mocked_page() {
    let request = mocked(
        "list_container_files",
        r#""container_id":"cntr_1""#,
        LIST_JSON,
    );
    let Answer::Json(text) = core().call(&wire(&request), Vec::new()).unwrap() else {
        panic!("a list answers JSON");
    };
    let page: ContainerFileListResponse = serde_json::from_str(&text).unwrap();
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].id, "cfile_1");
    assert_eq!(page.first_id.as_deref(), Some("cfile_1"));
    assert!(!page.has_more);
}

#[test]
fn call_upload_answers_the_mocked_file() {
    let request = mocked(
        "upload_container_file",
        concat!(
            r#""container_id":"cntr_1","#,
            r#""file":{"$file":{"name":"data.csv","blob":0,"content_type":"text/csv"}}"#,
        ),
        FILE_JSON,
    );
    let blobs = vec![b"a,b\n1,2\n".to_vec()];
    let Answer::Json(text) = core().call(&wire(&request), blobs).unwrap() else {
        panic!("an upload answers JSON");
    };
    let file: ContainerFileObject = serde_json::from_str(&text).unwrap();
    assert_eq!(file.id, "cfile_1");
    assert_eq!(file.path, "/mnt/data/a.txt");
}

#[test]
fn call_retrieve_answers_the_mocked_file() {
    let request = mocked(
        "retrieve_container_file",
        r#""container_id":"cntr_1","file_id":"cfile_1""#,
        FILE_JSON,
    );
    let Answer::Json(text) = core().call(&wire(&request), Vec::new()).unwrap() else {
        panic!("a retrieve answers JSON");
    };
    let file: ContainerFileObject = serde_json::from_str(&text).unwrap();
    assert_eq!(file.id, "cfile_1");
    assert_eq!(file.container_id, "cntr_1");
    assert_eq!(file.created_at, 1_750_000_000);
}

#[test]
fn call_delete_answers_the_mocked_deletion() {
    let request = mocked(
        "delete_container_file",
        r#""container_id":"cntr_1","file_id":"cfile_1""#,
        DELETED_JSON,
    );
    let Answer::Json(text) = core().call(&wire(&request), Vec::new()).unwrap() else {
        panic!("a delete answers JSON");
    };
    let deleted: DeleteContainerFileResponse = serde_json::from_str(&text).unwrap();
    assert_eq!(deleted.id, "cfile_1");
    assert!(deleted.deleted);
}

#[test]
fn call_refuses_an_operation_it_does_not_know() {
    let Err(failure) = core().call(&wire(r#"{"operation":"close_container_file"}"#), Vec::new())
    else {
        panic!("an unknown operation fails");
    };
    assert!(matches!(
        failure,
        SessionFailure::Invalid(ConvertError::Operation { .. })
    ));
}

#[test]
fn content_answers_the_mocked_json() {
    let request = mocked(
        "retrieve_container_file_content",
        r#""container_id":"cntr_1","file_id":"cfile_1""#,
        CONTENT_JSON,
    );
    let value = core().call(&wire(&request), Vec::new()).unwrap();
    let Answer::Json(text) = value else {
        panic!("a mocked content answers its JSON");
    };
    let parsed: Json = serde_json::from_str(&text).unwrap();
    let Json::Object(fields) = parsed else {
        panic!("the mock answers an object");
    };
    assert_eq!(
        fields.get("object"),
        Some(&Json::Text(String::from("file_content")))
    );
    assert_eq!(fields.get("text"), Some(&Json::Text(String::from("hello"))));
}

#[test]
fn asynchronous_dispatch_answers_a_mocked_list() {
    let request = mocked(
        "list_container_files",
        r#""container_id":"cntr_1""#,
        LIST_JSON,
    );
    let value = get_runtime()
        .block_on(core().dispatch(&wire(&request), Vec::new()))
        .unwrap();
    let Answer::Json(text) = value else {
        panic!("a list answers JSON");
    };
    let page: ContainerFileListResponse = serde_json::from_str(&text).unwrap();
    assert_eq!(page.data[0].id, "cfile_1");
}

#[rstest]
#[case::absent("")]
#[case::float_zero(r#","arguments":{"timeout":0.0}"#)]
#[case::negative(r#","arguments":{"timeout":-1}"#)]
#[case::positive(r#","arguments":{"timeout":4}"#)]
fn prepare_bounds_every_call_by_the_client_deadline(#[case] arguments: &str) {
    let timed = SessionCore::build(r#"{"environment":{"REQUEST_TIMEOUT":"30"}}"#).unwrap();
    let request = format!(r#"{{"operation":"list_container_files"{arguments}}}"#);
    let prepared = timed.prepare(&wire(&request), Vec::new()).unwrap();
    assert_eq!(prepared.handler.clients.deadline, Duration::from_secs(600));
}

#[test]
fn build_reads_an_api_key_only_when_it_is_not_empty() {
    let empty = SessionCore::build(r#"{"environment":{"OPENAI_API_KEY":""}}"#).unwrap();
    assert!(empty.env_api_key.is_none());
    let set = SessionCore::build(r#"{"environment":{"OPENAI_API_KEY":"sk-test"}}"#).unwrap();
    assert!(set.env_api_key.is_some());
}

#[test]
fn build_refuses_a_request_timeout_that_is_not_a_duration() {
    let Err(error) = SessionCore::build(r#"{"environment":{"REQUEST_TIMEOUT":"-5"}}"#) else {
        panic!("a negative request timeout has no duration");
    };
    assert!(matches!(
        error,
        SessionError::Json(ConvertError::Timeout { .. })
    ));
}

#[test]
fn error_provider_without_a_config_keeps_the_bad_request_class() {
    let request =
        r#"{"operation":"list_container_files","arguments":{"custom_llm_provider":"azure"}}"#;
    let Err(failure) = core().call(&wire(request), Vec::new()) else {
        panic!("a provider without a config fails");
    };
    let SessionFailure::Raised(payload) = failure else {
        panic!("a provider without a config raises the mapped error");
    };
    assert_eq!(payload.kind, "BadRequestError");
    assert_eq!(payload.status_code, Some(400));
    assert_eq!(payload.llm_provider.as_deref(), Some("azure"));
    assert!(payload.message.contains("azure"));
}

#[test]
fn error_text_answers_name_the_operation() {
    let empty = wire(r#"{"operation":"retrieve_container_file_content"}"#);
    let as_bytes = binary(&empty);
    assert!(matches!(
        as_bytes,
        SessionFailure::Invalid(ConvertError::Binary { operation })
            if operation == "retrieve_container_file_content"
    ));
}

#[test]
fn error_session_failure_wraps_a_setup_error() {
    let failure = SessionFailure::from(SessionError::from(ConvertError::Operation {
        operation: String::from("close_container_file"),
    }));
    assert!(matches!(failure, SessionFailure::Setup(_)));
}

#[test]
fn error_session_failure_wraps_an_invalid_error() {
    let failure = SessionFailure::from(ConvertError::Operation {
        operation: String::from("close_container_file"),
    });
    assert!(matches!(failure, SessionFailure::Invalid(_)));
}

#[test]
fn error_conversion_keeps_the_python_class() {
    Python::initialize();
    Python::attach(|py| {
        let Err(source) = serde_json::from_str::<ContainerFileListResponse>("not json") else {
            panic!("the text is not JSON");
        };
        let invalid = PyErr::from(ConvertError::Json { source });
        assert!(invalid.is_instance_of::<PyValueError>(py));
        let unknown = PyErr::from(ConvertError::Operation {
            operation: String::from("close_container_file"),
        });
        assert!(unknown.is_instance_of::<PyRuntimeError>(py));
    });
}

#[test]
fn error_session_error_keeps_the_python_class() {
    Python::initialize();
    Python::attach(|py| {
        let json = PyErr::from(SessionError::from(ConvertError::Operation {
            operation: String::from("close_container_file"),
        }));
        assert!(json.is_instance_of::<PyValueError>(py));
        let Err(source) = "not-a-number".parse::<f64>() else {
            panic!("the text is not a number");
        };
        let timeout = PyErr::from(SessionError::Timeout {
            source: TimeoutError::Invalid { source },
        });
        assert!(timeout.is_instance_of::<PyRuntimeError>(py));
    });
}

#[test]
fn json_text_answers_the_text_or_the_binary_error() {
    let empty = wire("{}");
    assert_eq!(
        json_text(Answer::Json(String::from("{}")), &empty).unwrap(),
        "{}"
    );
    let binary_answer = json_text(Answer::Bytes(vec![1, 2]), &empty);
    assert!(matches!(
        binary_answer,
        Err(SessionFailure::Invalid(ConvertError::Binary { .. }))
    ));
}

#[test]
fn answer_keeps_the_reply_variant() {
    let bytes = answer(ContainerFileReply::Bytes(vec![7, 8])).unwrap();
    assert!(matches!(bytes, Answer::Bytes(values) if values == vec![7, 8]));
    let file: ContainerFileObject = serde_json::from_str(FILE_JSON).unwrap();
    let encoded = answer(ContainerFileReply::File(file)).unwrap();
    let Answer::Json(text) = encoded else {
        panic!("a file answers JSON");
    };
    let parsed: ContainerFileObject = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed.id, "cfile_1");
}

#[test]
fn configs_answer_only_for_openai() {
    let settings = Raw::default().convert(&BTreeMap::new()).unwrap();
    let costs: Arc<dyn ContainerCosts> = Arc::new(stack::NoCosts);
    let configs = stack::Configs::new(
        settings,
        LiteLlmGlobals {
            api_base: None,
            api_key: None,
            openai_key: None,
            organization: None,
        },
        None,
        costs,
    );
    assert!(configs.container_config(ProviderId::Openai).is_some());
    assert!(configs.container_config(ProviderId::Azure).is_none());
    assert!(configs.container_config(ProviderId::AzureText).is_none());
    assert!(configs.container_config(ProviderId::Anthropic).is_none());
}

#[test]
fn no_costs_never_bills() {
    assert_eq!(
        stack::NoCosts.code_interpreter_cost(2, "openai"),
        Decimal::ZERO
    );
}

#[test]
fn decoder_reads_a_plain_container_id() {
    let decoded = stack::Decoder
        .decode("cntr_1", "openai", &Object::new())
        .unwrap();
    assert_eq!(decoded.container_id, "cntr_1");
    assert_eq!(decoded.custom_llm_provider, "openai");
}

#[test]
fn decoder_reads_a_managed_container_id() {
    let encoded = build_container_id(Some("azure"), Some("gpt-4o-mini"), "cntr_origin");
    let decoded = stack::Decoder
        .decode(&encoded, "openai", &Object::new())
        .unwrap();
    assert_eq!(decoded.container_id, "cntr_origin");
    assert_eq!(decoded.custom_llm_provider, "azure");
    assert_eq!(
        decoded.litellm_params.get("model_id"),
        Some(&serde_json::json!("gpt-4o-mini"))
    );
}

#[test]
fn params_field_names_the_litellm_params() {
    assert_eq!(stack::params_field().field, "litellm_params");
}

#[test]
fn init_carries_the_call_type_and_the_call_id() {
    let given = stack::init(
        "list_container_files",
        Some("call_1"),
        &stack::SessionIds,
        &stack::SessionClock::new(),
    );
    assert_eq!(given.call_type, "list_container_files");
    assert_eq!(given.litellm_call_id, "call_1");
    assert!(given.start_time.timestamp() > 1_600_000_000);
    let generated = stack::init(
        "upload_container_file",
        None,
        &stack::SessionIds,
        &stack::SessionClock::new(),
    );
    assert_eq!(generated.call_type, "upload_container_file");
    assert_eq!(
        Uuid::parse_str(&generated.litellm_call_id)
            .unwrap()
            .get_version_num(),
        4
    );
}

#[test]
fn logging_state_carries_the_call_type_and_the_call_id() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let state = stack::logging_state(
        "delete_container_file",
        Some("call_1"),
        &redactor,
        &stack::SessionIds,
        &stack::SessionClock::new(),
    )
    .unwrap();
    let locked = state.lock().unwrap();
    assert_eq!(locked.call_type, "delete_container_file");
    assert_eq!(locked.litellm_call_id, "call_1");
}

#[test]
fn session_clock_counts_the_time_since_it_was_built() {
    let clock = stack::SessionClock::new();
    get_runtime().block_on(async {
        let started = clock.monotonic();
        sleep(Duration::from_millis(2)).await;
        assert!(clock.monotonic() > started);
    });
}

#[test]
fn register_adds_the_file_classes() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        completion::register(&module).unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeContainerFileSession").unwrap());
        assert!(module.hasattr("NativeContainerFileError").unwrap());
        assert!(module.hasattr("NativeError").unwrap());
    });
}

#[test]
fn native_error_carries_the_payload_as_the_exception_text() {
    Python::initialize();
    Python::attach(|py| {
        let error_type =
            PyErr::new_type(py, c"probe.NativeContainerFileError", None, None, None).unwrap();
        let payload = NativeFilePayload {
            kind: String::from("NotFoundError"),
            message: String::from("no such container file"),
            status_code: Some(404),
            llm_provider: Some(String::from("openai")),
            model: None,
            body: None,
            headers: BTreeMap::new(),
        };
        let error = native_error(py, &error_type, &payload);
        assert!(error.to_string().contains("no such container file"));
    });
}

#[test]
fn raised_failure_keeps_the_class_as_the_exception_text() {
    let request =
        r#"{"operation":"list_container_files","arguments":{"custom_llm_provider":"azure"}}"#;
    let Err(failure) = core().call(&wire(request), Vec::new()) else {
        panic!("a provider without a config fails");
    };
    Python::initialize();
    let text = Python::attach(|py| {
        let error_type =
            PyErr::new_type(py, c"probe.NativeContainerFileError", None, None, None).unwrap();
        failure.into_pyerr(py, &error_type).to_string()
    });
    assert!(text.contains("BadRequestError"));
    assert!(text.contains("azure"));
}

#[test]
fn session_call_answers_a_mocked_list() {
    Python::initialize();
    let request = mocked(
        "list_container_files",
        r#""container_id":"cntr_1""#,
        LIST_JSON,
    );
    let text = Python::attach(|py| session(py).call(py, &request, Vec::new())).unwrap();
    assert!(text.contains("cfile_1"));
}

#[test]
fn session_call_raises_a_value_error_for_a_request_that_is_not_json() {
    Python::initialize();
    Python::attach(|py| {
        let error = session(py).call(py, "not json", Vec::new()).unwrap_err();
        assert!(error.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn session_content_answers_the_mocked_json() {
    Python::initialize();
    let request = mocked(
        "retrieve_container_file_content",
        r#""container_id":"cntr_1","file_id":"cfile_1""#,
        CONTENT_JSON,
    );
    Python::attach(|py| {
        let answer = session(py).content(py, &request).unwrap();
        let object = answer.bind(py).cast::<PyDict>().unwrap();
        let kind: String = object
            .get_item("object")
            .unwrap()
            .unwrap()
            .extract()
            .unwrap();
        let text: String = object.get_item("text").unwrap().unwrap().extract().unwrap();
        assert_eq!(kind, "file_content");
        assert_eq!(text, "hello");
    });
}
