#![cfg(test)]

use std::collections::BTreeMap;
use std::ffi::CStr;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::{batch_provider_list, xai_identity};
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::batch_provider_list::{BatchProviderList, BatchProviderListInput};
use betterlite_core::api::resource_sdk_settings::ResourceSdkSettings;
use betterlite_core::api::resource_sdk_settings::timeout::{FineTuningTimeout, StructuredTimeout};
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::api::batch_records::WireDecodeLimits;
use betterlite_openai::batches::error::{BatchesError, SdkError};
use betterlite_openai::batches::sdk::{Timeout, TimeoutError, TimeoutPhases};
use betterlite_openai::xai_batches::error::XaiBatchesError;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use pyo3_async_runtimes::tokio::get_runtime;
use rstest::rstest;

use super::convert::{CallMode, ModuleSettings, cancel_payload, parse, section_settings};
use super::error::{DispatchFailure, ListingError, payload_for};
use super::register;
use super::session::ListingCore;
use super::transport::{ListTransport, ProviderFailure, dispatch_failure, sdk_timeout};

const EMPTY_ENVIRONMENT: &str = r#"{"environment":{}}"#;
const SECRET_ENVIRONMENT: &str = r#"{"environment":{},"settings":{"api_key":"sk-module","openai_key":"sk-openai","azure_key":"sk-azure"}}"#;
// no listener answers port 1, so a session's provider call fails without any network behind it
const LIST_OPENAI_REFUSED: &str =
    r#"{"provider":"openai","api_base":"http://127.0.0.1:1/v1","api_key":"sk-test"}"#;
const LIST_XAI_REFUSED: &str =
    r#"{"provider":"xai","api_base":"http://127.0.0.1:1","api_key":"sk-test"}"#;
const CANCEL_OPENAI_REFUSED: &str = r#"{"cancel":{"batch_id":"batch-1","provider":"openai","params":{"api_base":"http://127.0.0.1:1/v1","api_key":"sk-test"}}}"#;
const CANCEL_XAI_REFUSED: &str = r#"{"cancel":{"batch_id":"batch-1","provider":"xai","params":{"api_base":"http://127.0.0.1:1","api_key":"sk-test"}}}"#;

fn core() -> ListingCore {
    ListingCore::build(EMPTY_ENVIRONMENT).unwrap()
}

fn proxied() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

const fn decode_limits() -> WireDecodeLimits {
    WireDecodeLimits {
        max_json_bytes: 1024,
        max_event_bytes: 1024,
        max_items: 16,
    }
}

fn transport() -> ListTransport {
    ListTransport {
        client: proxied(),
        xai: xai_identity::Settings::default(),
        limits: decode_limits(),
    }
}

fn default_settings() -> batch_provider_list::Settings {
    section_settings::<batch_provider_list::Raw>(&ModuleSettings::default(), &BTreeMap::new())
        .unwrap()
}

#[test]
fn register_adds_the_batch_listing_classes() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeBatchListSession").unwrap());
        assert!(module.hasattr("NativeBatchListError").unwrap());
    });
}

// every sync session error names the class, the provider and the model the payload asked for
#[rstest]
#[case::list_openai_refused("list", LIST_OPENAI_REFUSED, "openai", "APIConnectionError", None)]
#[case::list_xai_refused("list", LIST_XAI_REFUSED, "xai", "APIConnectionError", None)]
#[case::cancel_openai_refused(
    "cancel",
    CANCEL_OPENAI_REFUSED,
    "openai",
    "APIConnectionError",
    None
)]
#[case::cancel_xai_refused("cancel", CANCEL_XAI_REFUSED, "xai", "APIConnectionError", None)]
#[case::list_azure("list", r#"{"provider":"azure"}"#, "azure", "BadRequestError", None)]
#[case::list_vertex(
    "list",
    r#"{"provider":"vertex_ai"}"#,
    "vertex_ai",
    "BadRequestError",
    None
)]
#[case::cancel_azure(
    "cancel",
    r#"{"cancel":{"batch_id":"batch-1","provider":"azure"}}"#,
    "azure",
    "BadRequestError",
    None
)]
#[case::cancel_vertex(
    "cancel",
    r#"{"cancel":{"batch_id":"batch-1","provider":"vertex_ai"}}"#,
    "vertex_ai",
    "BadRequestError",
    None
)]
#[case::cancel_bedrock(
    "cancel",
    r#"{"cancel":{"batch_id":"batch-1","provider":"bedrock"}}"#,
    "bedrock",
    "BadRequestError",
    None
)]
#[case::list_unknown(
    "list",
    r#"{"provider":"cohere"}"#,
    "cohere",
    "BadRequestError",
    Some("n/a")
)]
#[case::cancel_unknown(
    "cancel",
    r#"{"cancel":{"batch_id":"batch-1","provider":"cohere"}}"#,
    "cohere",
    "BadRequestError",
    Some("n/a")
)]
fn session_errors_name_the_provider_and_model(
    #[case] method: &str,
    #[case] payload: &str,
    #[case] provider: &str,
    #[case] error_type: &str,
    #[case] model: Option<&str>,
) {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        let class = module.getattr("NativeBatchListSession").unwrap();
        let session = class.call1((EMPTY_ENVIRONMENT,)).unwrap();
        let error = session
            .call_method1(method, (payload, Vec::<Vec<u8>>::new()))
            .unwrap_err();
        let message = error.value(py).to_string();
        let wanted = [
            format!(r#""type":"{error_type}""#),
            format!(r#""llm_provider":"{provider}""#),
            model.map_or_else(
                || String::from(r#""model":null"#),
                |name| format!(r#""model":"{name}""#),
            ),
        ];
        for part in wanted {
            assert!(message.contains(&part), "{method}: {message}");
        }
    });
}

#[rstest]
#[case::list("list")]
#[case::cancel("cancel")]
fn session_bad_json_is_a_value_error(#[case] method: &str) {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        let class = module.getattr("NativeBatchListSession").unwrap();
        let session = class.call1((EMPTY_ENVIRONMENT,)).unwrap();
        let error = session
            .call_method1(method, ("not json", Vec::<Vec<u8>>::new()))
            .unwrap_err();
        assert!(error.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn session_bad_settings_text_is_a_value_error() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        let class = module.getattr("NativeBatchListSession").unwrap();
        let error = class.call1(("not json",)).unwrap_err();
        assert!(error.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn session_accepts_the_module_secrets() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        let class = module.getattr("NativeBatchListSession").unwrap();
        class.call1((SECRET_ENVIRONMENT,)).unwrap();
    });
}

#[test]
fn session_rejects_an_async_invoke_status() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        let class = module.getattr("NativeBatchListSession").unwrap();
        let session = class.call1((EMPTY_ENVIRONMENT,)).unwrap();
        let payload =
            r#"{"async_invoke":{"invocationArn":"arn:aws:batch:1","modelArn":"arn:aws:model:1"}}"#;
        let error = session
            .call_method1("cancel", (payload, Vec::<Vec<u8>>::new()))
            .unwrap_err();
        assert!(error.is_instance_of::<PyValueError>(py));
    });
}

// the native async twins need the running loop the facade calls them from
const AWAIT_METHOD: &CStr = c"import asyncio, inspect
async def main():
    answer = getattr(session, method)(payload, [])
    awaitable = inspect.isawaitable(answer)
    try:
        await answer
    except Exception as error:
        return (awaitable, type(error).__name__, str(error))
    return (awaitable, 'no error', '')
answer = asyncio.run(main())
";

// list_batches with alist_batches and acancel_batch both answer an awaitable, not the answer
#[rstest]
#[case::alist("alist", LIST_OPENAI_REFUSED)]
#[case::acancel("acancel", CANCEL_OPENAI_REFUSED)]
fn session_async_methods_answer_with_a_coroutine(#[case] method: &str, #[case] payload: &str) {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        let class = module.getattr("NativeBatchListSession").unwrap();
        let session = class.call1((EMPTY_ENVIRONMENT,)).unwrap();
        let globals = PyDict::new(py);
        globals.set_item("session", session).unwrap();
        globals.set_item("method", method).unwrap();
        globals.set_item("payload", payload).unwrap();
        py.run(AWAIT_METHOD, Some(&globals), None).unwrap();
        let answer = globals.get_item("answer").unwrap().unwrap();
        let (awaitable, kind, text): (bool, String, String) = answer.extract().unwrap();
        assert!(awaitable, "{method} must answer an awaitable");
        assert_eq!(kind, "NativeBatchListError", "{method}: {text}");
        assert!(
            text.contains(r#""type":"APIConnectionError""#),
            "{method}: {text}"
        );
    });
}

#[rstest]
#[case::azure("azure")]
#[case::vertex("vertex_ai")]
fn transport_refuses_an_unwired_list_provider(#[case] provider: &str) {
    let service = BatchProviderList {
        settings: ResourceSdkSettings::from(default_settings()),
    };
    let text = format!(r#"{{"provider":"{provider}"}}"#);
    let input: BatchProviderListInput = parse(&text).unwrap();
    let result = service.run(&input).unwrap();
    let outcome = get_runtime().block_on(transport().list(&result.call, result.is_async));
    match outcome {
        Err(DispatchFailure::Unsupported { provider: name }) => {
            assert_eq!(name, provider);
        }
        Ok(_) | Err(_) => panic!("the {provider} branch is not wired in this build"),
    }
}

#[test]
fn transport_sdk_timeout_keeps_the_seconds() {
    assert_eq!(
        sdk_timeout(&FineTuningTimeout::Seconds(3.0_f64)),
        Timeout::Seconds(3.0_f64)
    );
    let structured = StructuredTimeout {
        connect: Some(1.0_f64),
        read: Some(2.0_f64),
        write: None,
        pool: None,
    };
    assert_eq!(
        sdk_timeout(&FineTuningTimeout::Structured(structured)),
        Timeout::Phases(TimeoutPhases {
            connect: Some(1.0_f64),
            read: Some(2.0_f64),
            write: None,
            pool: None,
        })
    );
}

#[test]
fn transport_failures_follow_the_status() {
    let sdk_status = SdkError::Status {
        status: 429,
        error_type: None,
        param: None,
        code: None,
        message: "slow down".to_owned(),
    };
    assert!(matches!(
        dispatch_failure(ProviderFailure::OpenAi(&BatchesError::Sdk { source: sdk_status })),
        DispatchFailure::Status {
            status: 429,
            message,
            ..
        } if message == "slow down"
    ));
    assert!(matches!(
        dispatch_failure(ProviderFailure::OpenAi(&BatchesError::NotInitialized)),
        DispatchFailure::Connection(_)
    ));
    assert!(matches!(
        dispatch_failure(ProviderFailure::Xai(&XaiBatchesError::MissingApiKey)),
        DispatchFailure::Status { status: 401, .. }
    ));
    assert!(matches!(
        dispatch_failure(ProviderFailure::Xai(&XaiBatchesError::InputFileRequired)),
        DispatchFailure::Status { status: 400, .. }
    ));
}

// a provider status hands the SDK the error object's type, param and code, not only the status and message
#[test]
fn transport_failure_keeps_the_provider_fields() {
    let sdk_status = SdkError::Status {
        status: 400,
        error_type: Some("invalid_request_error".to_owned()),
        param: Some("after".to_owned()),
        code: Some("invalid_cursor".to_owned()),
        message: "bad cursor".to_owned(),
    };
    match dispatch_failure(ProviderFailure::OpenAi(&BatchesError::Sdk {
        source: sdk_status,
    })) {
        DispatchFailure::Status {
            status: 400,
            body: Some(body),
            ..
        } => {
            assert_eq!(body.error_type.as_deref(), Some("invalid_request_error"));
            assert_eq!(body.param.as_deref(), Some("after"));
            assert_eq!(body.code.as_deref(), Some("invalid_cursor"));
        }
        other @ (DispatchFailure::Status { .. }
        | DispatchFailure::Connection(_)
        | DispatchFailure::Timeout(_)
        | DispatchFailure::Unsupported { .. }) => {
            panic!("expected a provider status, got {other:?}")
        }
    }
}

// an error object that names only one of the three fields still reaches the exception
#[test]
fn transport_failure_keeps_a_partial_body() {
    let sdk_status = SdkError::Status {
        status: 400,
        error_type: None,
        param: None,
        code: Some("invalid_cursor".to_owned()),
        message: "bad cursor".to_owned(),
    };
    match dispatch_failure(ProviderFailure::OpenAi(&BatchesError::Sdk {
        source: sdk_status,
    })) {
        DispatchFailure::Status {
            body: Some(body), ..
        } => {
            assert_eq!(body.code.as_deref(), Some("invalid_cursor"));
            assert_eq!(body.error_type, None);
            assert_eq!(body.param, None);
        }
        other @ (DispatchFailure::Status { .. }
        | DispatchFailure::Connection(_)
        | DispatchFailure::Timeout(_)
        | DispatchFailure::Unsupported { .. }) => {
            panic!("expected a provider status, got {other:?}")
        }
    }
}

#[test]
fn list_with_reports_a_core_timeout() {
    let input: BatchProviderListInput = parse(r#"{"provider":"openai","timeout":"abc"}"#).unwrap();
    let error = get_runtime()
        .block_on(core().list_with(input, CallMode::Sync))
        .unwrap_err();
    let payload = payload_for(&error, "openai", None).unwrap();
    assert_eq!(payload.error_type, "BadRequestError");
    assert_eq!(payload.status_code, Some(400));
}

#[test]
fn cancel_with_reports_a_core_timeout() {
    let text =
        r#"{"cancel":{"batch_id":"batch-1","provider":"openai","params":{"timeout":"abc"}}}"#;
    let call = cancel_payload(text).unwrap();
    let error = get_runtime()
        .block_on(core().cancel_with(call, CallMode::Sync))
        .unwrap_err();
    let payload = payload_for(&error, "openai", None).unwrap();
    assert_eq!(payload.error_type, "BadRequestError");
    assert_eq!(payload.status_code, Some(400));
}

// the shared TimeoutError reaches the Python exception as a 400, the deviation B10146 records
#[test]
fn payload_maps_a_timeout_failure() {
    let error = ListingError::Dispatch(DispatchFailure::Timeout(TimeoutError::Invalid {
        seconds: -1.0_f64,
    }));
    let payload = payload_for(&error, "xai", None).unwrap();
    assert_eq!(payload.error_type, "BadRequestError");
    assert_eq!(payload.status_code, Some(400));
    assert!(payload.message.contains("-1"));
}
