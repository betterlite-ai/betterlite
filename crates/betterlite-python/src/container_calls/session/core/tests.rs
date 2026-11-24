#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use crate::{completion, container_calls};
use betterlite_config::sections::Section as _;
use betterlite_config::sections::openai_credentials::Raw;
use betterlite_core::api::container_file_endpoints::handler::{
    ContainerConfigLookup as _, ContainerIdDecoder as _,
};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::container_contracts::records::{
    ContainerListResponse, ContainerObject, DeleteContainerResult,
};
use betterlite_openai::containers::costs::ContainerCosts;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::openai_base_config::Object;
use betterlite_sdk::call_wrapper::ports::WrapperClock as _;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::get_runtime;
use rust_decimal::Decimal;
use uuid::Uuid;

use super::super::NativeContainerSession;
use super::asynchronous;
use super::{CollectionCall, SessionCore, encode, encode_created, resource_call_type};
use crate::container_calls::convert::{CallWire, ConvertError, NativeContainerPayload, parse};
use crate::container_calls::error::{SessionError, SessionFailure, native_error};
use crate::container_calls::stack;

const CONTAINER: &str = r#"{"id":"cntr_1","object":"container","created_at":1,"status":"running",
    "expires_after":null,"last_active_at":null,"name":"box"}"#;
const PAGE: &str = r#"{"object":"list","data":[],"first_id":null,"last_id":null,"has_more":false}"#;
const DELETED: &str = r#"{"id":"cntr_1","object":"container.deleted","deleted":true}"#;

fn core() -> SessionCore {
    // an explicit empty environment keeps the test off the process's proxy and credential variables
    SessionCore::build(r#"{"environment":{}}"#).unwrap()
}

fn wire(text: &str) -> CallWire {
    parse(text).unwrap()
}

fn mocked(operation: &str, arguments: &str, answer: &str) -> String {
    format!(
        r#"{{"operation":"{operation}","arguments":{{{arguments},"mock_response":{}}}}}"#,
        serde_json::to_string(answer).unwrap()
    )
}

#[test]
fn call_create_answers_the_mocked_container_with_its_hidden_params() {
    let request = mocked("create_container", r#""name":"box""#, CONTAINER);
    let answer = core().call(&wire(&request)).unwrap();
    let container: ContainerObject = serde_json::from_str(&answer).unwrap();
    assert_eq!(container.id, "cntr_1");
    assert_eq!(container.name.as_deref(), Some("box"));
    assert!(answer.contains(r#""_hidden_params":"#));
}

#[test]
fn call_list_answers_the_mocked_page() {
    let request = mocked("list_containers", r#""limit":2"#, PAGE);
    let answer = core().call(&wire(&request)).unwrap();
    let page: ContainerListResponse = serde_json::from_str(&answer).unwrap();
    assert!(page.data.is_empty());
    assert!(!page.has_more);
}

#[test]
fn call_retrieve_answers_the_mocked_container() {
    let request = mocked(
        "retrieve_container",
        r#""container_id":"cntr_1""#,
        CONTAINER,
    );
    let answer = core().call(&wire(&request)).unwrap();
    let container: ContainerObject = serde_json::from_str(&answer).unwrap();
    assert_eq!(container.id, "cntr_1");
    assert_eq!(container.status, "running");
}

#[test]
fn call_delete_answers_the_mocked_deletion() {
    let request = mocked("delete_container", r#""container_id":"cntr_1""#, DELETED);
    let answer = core().call(&wire(&request)).unwrap();
    let deleted: DeleteContainerResult = serde_json::from_str(&answer).unwrap();
    assert_eq!(deleted.id, "cntr_1");
    assert!(deleted.deleted);
}

#[test]
fn asynchronous_call_answers_a_mocked_create() {
    let request = mocked("create_container", r#""name":"box""#, CONTAINER);
    let answer = get_runtime()
        .block_on(asynchronous::acall(&core(), &wire(&request)))
        .unwrap();
    assert!(answer.contains("cntr_1"));
}

#[test]
fn asynchronous_call_answers_a_mocked_delete() {
    let request = mocked("delete_container", r#""container_id":"cntr_1""#, DELETED);
    let answer = get_runtime()
        .block_on(asynchronous::acall(&core(), &wire(&request)))
        .unwrap();
    let deleted: DeleteContainerResult = serde_json::from_str(&answer).unwrap();
    assert!(deleted.deleted);
}

#[test]
fn call_refuses_an_operation_it_does_not_know() {
    let answer = core().call(&wire(r#"{"operation":"close_container"}"#));
    assert!(matches!(
        answer,
        Err(SessionFailure::Invalid(ConvertError::Operation { .. }))
    ));
}

#[test]
fn deadline_takes_the_request_timeout_for_zero_and_none() {
    let session = SessionCore::build(r#"{"environment":{"REQUEST_TIMEOUT":"30"}}"#).unwrap();
    assert_eq!(session.deadline(None).unwrap(), Duration::from_secs(30));
    assert_eq!(
        session.deadline(Some(0.0_f64)).unwrap(),
        Duration::from_secs(30)
    );
    assert_eq!(
        session.deadline(Some(2.5_f64)).unwrap(),
        Duration::try_from_secs_f64(2.5_f64).unwrap()
    );
}

#[test]
fn operation_deadline_takes_the_request_timeout_for_zero_and_none() {
    let session = SessionCore::build(r#"{"environment":{"REQUEST_TIMEOUT":"30"}}"#).unwrap();
    assert_eq!(session.operation_deadline(None), Duration::from_secs(30));
    assert_eq!(
        session.operation_deadline(Some(Duration::ZERO)),
        Duration::from_secs(30)
    );
    assert_eq!(
        session.operation_deadline(Some(Duration::from_secs(4))),
        Duration::from_secs(4)
    );
}

#[test]
fn collection_call_names_the_logging_type_the_call_id_and_the_timeout() {
    let create = wire(r#"{"arguments":{"timeout":2.0,"litellm_call_id":"call_1"}}"#)
        .arguments
        .create()
        .unwrap();
    let list = wire(r#"{"arguments":{"timeout":null}}"#)
        .arguments
        .list()
        .unwrap();
    assert_eq!(
        CollectionCall::Create(&create).call_type(),
        "create_container"
    );
    assert_eq!(CollectionCall::List(&list).call_type(), "list_containers");
    assert_eq!(CollectionCall::Create(&create).call_id(), Some("call_1"));
    assert_eq!(CollectionCall::List(&list).call_id(), None);
    assert_eq!(CollectionCall::Create(&create).timeout(), Some(2.0_f64));
    assert_eq!(CollectionCall::List(&list).timeout(), None);
}

#[test]
fn resource_call_type_names_retrieve_and_delete() {
    assert_eq!(resource_call_type(false), "retrieve_container");
    assert_eq!(resource_call_type(true), "delete_container");
}

#[test]
fn encode_writes_the_value_as_the_answer_text() {
    let deletion = BTreeMap::from([("deleted", true)]);
    assert_eq!(encode(&deletion).unwrap(), r#"{"deleted":true}"#);
}

#[test]
fn encode_created_adds_the_hidden_params_to_the_record() {
    let container: ContainerObject = serde_json::from_str(CONTAINER).unwrap();
    let answer = encode_created(&container).unwrap();
    let record: ContainerObject = serde_json::from_str(&answer).unwrap();
    assert_eq!(record.id, "cntr_1");
    assert!(answer.contains(r#""_hidden_params":"#));
}

#[test]
fn build_reads_an_api_key_only_when_it_is_not_empty() {
    let empty = SessionCore::build(r#"{"environment":{"OPENAI_API_KEY":""}}"#).unwrap();
    assert!(empty.env_api_key.is_none());
    let set = SessionCore::build(r#"{"environment":{"OPENAI_API_KEY":"sk-test"}}"#).unwrap();
    assert!(set.env_api_key.is_some());
}

#[test]
fn params_field_names_the_litellm_params() {
    assert_eq!(stack::params_field().field, "litellm_params");
}

#[test]
fn init_carries_the_call_type_and_the_call_id() {
    let given = stack::init(
        "create_container",
        Some("call_1"),
        &stack::SessionIds,
        &stack::SessionClock::new(),
    );
    assert_eq!(given.call_type, "create_container");
    assert_eq!(given.litellm_call_id, "call_1");
    assert!(given.start_time.timestamp() > 1_600_000_000);
    let generated = stack::init(
        "list_containers",
        None,
        &stack::SessionIds,
        &stack::SessionClock::new(),
    );
    assert_eq!(generated.call_type, "list_containers");
    assert_eq!(
        Uuid::parse_str(&generated.litellm_call_id)
            .unwrap()
            .get_version_num(),
        4
    );
}

#[test]
fn session_clock_counts_the_time_since_it_was_built() {
    let clock = stack::SessionClock::new();
    get_runtime().block_on(tokio::time::sleep(Duration::from_millis(2)));
    assert!(clock.monotonic() > Duration::ZERO);
}

#[test]
fn catalog_costs_debug_names_the_type() {
    let costs = stack::CatalogCosts::new(&BTreeMap::new()).unwrap();
    assert!(format!("{costs:?}").contains("CatalogCosts"));
}

#[test]
fn catalog_costs_reads_the_openai_code_interpreter_fee() {
    let costs = stack::CatalogCosts::new(&BTreeMap::new()).unwrap();
    assert_eq!(costs.code_interpreter_cost(1, "openai"), Decimal::new(3, 2));
}

#[test]
fn decoder_reads_a_plain_container_id() {
    let decoded = stack::Decoder
        .decode("cntr_1", "openai", &Object::new())
        .unwrap();
    assert_eq!(decoded.container_id, "cntr_1");
    assert_eq!(decoded.custom_llm_provider, "openai");
}

#[derive(Debug)]
struct FreeCosts;

impl ContainerCosts for FreeCosts {
    fn code_interpreter_cost(&self, _sessions: u32, _provider: &str) -> Decimal {
        Decimal::ZERO
    }
}

#[test]
fn configs_answer_only_for_openai() {
    let settings = Raw::default().convert(&BTreeMap::new()).unwrap();
    let configs = stack::Configs::new(
        settings,
        LiteLlmGlobals {
            api_base: None,
            api_key: None,
            openai_key: None,
            organization: None,
        },
        None,
        Arc::new(FreeCosts),
    );
    assert!(configs.container_config(ProviderId::Openai).is_some());
    // the azure container config is not ported yet, so both azure ids answer no config
    assert!(configs.container_config(ProviderId::Azure).is_none());
    assert!(configs.container_config(ProviderId::AzureText).is_none());
    assert!(configs.container_config(ProviderId::Anthropic).is_none());
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
fn register_adds_the_container_classes() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        completion::register(&module).unwrap();
        container_calls::register(&module).unwrap();
        assert!(module.hasattr("NativeContainerSession").unwrap());
        assert!(module.hasattr("NativeContainerError").unwrap());
    });
}

#[test]
fn native_error_carries_the_payload_as_the_exception_text() {
    Python::initialize();
    Python::attach(|py| {
        let error_type =
            PyErr::new_type(py, c"probe.NativeContainerError", None, None, None).unwrap();
        let payload = NativeContainerPayload {
            kind: String::from("NotFoundError"),
            message: String::from("no such container"),
            status_code: Some(404),
            llm_provider: None,
            model: None,
            body: None,
            headers: BTreeMap::new(),
        };
        let error = native_error(py, &error_type, &payload);
        assert!(error.to_string().contains("no such container"));
    });
}

#[test]
fn session_call_answers_a_mocked_create() {
    Python::initialize();
    let request = mocked("create_container", r#""name":"box""#, CONTAINER);
    let answer = Python::attach(|py| {
        let session = NativeContainerSession {
            core: Arc::new(core()),
            error_type: PyErr::new_type(py, c"probe.NativeContainerError", None, None, None)
                .unwrap(),
        };
        session.call(py, &request, &py.None().into_bound(py).into_any())
    })
    .unwrap();
    assert!(answer.contains("cntr_1"));
}
