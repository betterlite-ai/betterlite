#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::batch_provider_list;
use betterlite_core::api::batch_provider_cancel::connection::CancelTimeout;
use betterlite_core::api::batch_provider_cancel::{BatchProviderCancelError, CancelCall};
use betterlite_core::api::batch_provider_list::BatchListTimeoutValue;
use betterlite_core::api::batch_provider_list::error::BatchProviderListError;
use betterlite_core::api::resource_sdk_settings::timeout::{TimeoutError, TimeoutValue};
use rstest::rstest;

use super::{
    CallMode, ConvertError, HiddenParams, ModuleSettings, cancel_input, cancel_payload, list_input,
    list_payload, section_settings,
};
use crate::batch_listing::error::{DispatchFailure, ErrorBody};
use crate::batch_listing::payload::{ProviderError, class_for_status, provider_error_payload};

const CANCEL: &str = r#"{"cancel":{"batch_id":"batch-1","model":"gpt-4o","provider":"openai","extra_headers":{"x-test":"1"},"extra_body":{"note":"n"},"params":{"api_base":"http://127.0.0.1:1/v1","api_key":"sk-test","timeout":3}}}"#;

#[test]
fn request_list_payload_defaults_to_openai_and_no_window() {
    let input = list_payload("{}").unwrap();
    assert_eq!(input.provider.name(), "openai");
    assert_eq!(input.after, None);
    assert_eq!(input.limit, None);
    assert!(input.timeout.is_none());
    assert!(!input.alist_batches);
}

#[test]
fn request_list_payload_reads_the_window() {
    let input = list_payload(r#"{"after":"batch-0","limit":0,"timeout":2.5}"#).unwrap();
    assert_eq!(input.after.as_deref(), Some("batch-0"));
    // a limit of zero is a value, not an absent one
    assert_eq!(input.limit, Some(0));
    assert!(matches!(
        input.timeout,
        Some(BatchListTimeoutValue::Seconds(seconds)) if seconds.total_cmp(&2.5_f64).is_eq()
    ));
}

#[test]
fn request_list_payload_keeps_a_text_timeout() {
    let input = list_payload(r#"{"timeout":"1.5"}"#).unwrap();
    assert!(matches!(
        input.timeout,
        Some(BatchListTimeoutValue::Text(text)) if text == "1.5"
    ));
}

#[test]
fn request_list_payload_reads_the_phases() {
    let input = list_payload(r#"{"timeout":{"connect":1,"read":2,"write":3,"pool":4}}"#).unwrap();
    match input.timeout {
        Some(BatchListTimeoutValue::Phases(phases)) => {
            assert_eq!(phases.connect, Some(1.0_f64));
            assert_eq!(phases.read, Some(2.0_f64));
            assert_eq!(phases.write, Some(3.0_f64));
            assert_eq!(phases.pool, Some(4.0_f64));
        }
        other => panic!("expected phases, got {other:?}"),
    }
}

#[test]
fn request_list_payload_drops_a_null_timeout() {
    let input = list_payload(r#"{"timeout":null}"#).unwrap();
    assert!(input.timeout.is_none());
}

#[test]
fn request_list_payload_rejects_a_timeout_shape() {
    assert!(matches!(
        list_payload(r#"{"timeout":true}"#),
        Err(ConvertError::Shapes(_))
    ));
}

#[test]
fn request_list_payload_rejects_a_phase_that_is_not_a_number() {
    assert!(matches!(
        list_payload(r#"{"timeout":{"read":"x"}}"#),
        Err(ConvertError::Phases(_))
    ));
}

#[test]
fn request_list_payload_rejects_an_unknown_key() {
    assert!(matches!(
        list_payload(r#"{"nonsense":1}"#),
        Err(ConvertError::Json { .. })
    ));
}

#[test]
fn request_list_input_marks_the_async_call() {
    let input = list_payload("{}").unwrap();
    assert!(list_input(input, CallMode::Async).alist_batches);
    let sync_input = list_payload("{}").unwrap();
    assert!(!list_input(sync_input, CallMode::Sync).alist_batches);
}

#[test]
fn request_cancel_payload_keeps_the_call() {
    let call = cancel_payload(CANCEL).unwrap();
    assert_eq!(call.batch_id, "batch-1");
    assert_eq!(call.model.as_deref(), Some("gpt-4o"));
    assert_eq!(call.provider.0, "openai");
    let headers = call.extra_headers.unwrap();
    assert_eq!(headers.get("x-test").map(String::as_str), Some("1"));
    assert_eq!(
        call.extra_body
            .as_ref()
            .and_then(|body| body.get("note"))
            .map(String::as_str),
        Some("n")
    );
    assert_eq!(
        call.params.api_base.as_deref(),
        Some("http://127.0.0.1:1/v1")
    );
    assert!(matches!(
        call.params.timeout,
        Some(CancelTimeout::Plain(TimeoutValue::Number(seconds)))
            if seconds.total_cmp(&3.0_f64).is_eq()
    ));
    assert!(matches!(call.call, CancelCall::Sync));
}

#[test]
fn request_cancel_payload_reads_a_structured_timeout() {
    let text = r#"{"cancel":{"batch_id":"batch-1","provider":"openai","params":{"timeout":{"connect":1,"read":2.5}}}}"#;
    let call = cancel_payload(text).unwrap();
    match call.params.timeout {
        Some(CancelTimeout::Structured(phases)) => {
            assert_eq!(phases.connect, Some(1.0_f64));
            assert_eq!(phases.read, Some(2.5_f64));
            assert_eq!(phases.write, None);
        }
        other => panic!("expected a structured timeout, got {other:?}"),
    }
}

#[test]
fn request_cancel_payload_reads_a_text_timeout() {
    let text = r#"{"cancel":{"batch_id":"batch-1","provider":"openai","params":{"timeout":"4"}}}"#;
    let call = cancel_payload(text).unwrap();
    assert!(matches!(
        call.params.timeout,
        Some(CancelTimeout::Plain(TimeoutValue::String(given))) if given == "4"
    ));
}

#[test]
fn request_cancel_payload_drops_a_null_timeout() {
    let text = r#"{"cancel":{"batch_id":"batch-1","provider":"openai","params":{"timeout":null}}}"#;
    let call = cancel_payload(text).unwrap();
    assert!(call.params.timeout.is_none());
}

#[test]
fn request_cancel_payload_rejects_a_timeout_shape() {
    let text = r#"{"cancel":{"batch_id":"batch-1","provider":"openai","params":{"timeout":[]}}}"#;
    assert!(matches!(cancel_payload(text), Err(ConvertError::Shapes(_))));
}

#[test]
fn request_cancel_payload_rejects_a_phase_that_is_not_a_number() {
    let text = r#"{"cancel":{"batch_id":"batch-1","provider":"openai","params":{"timeout":{"read":"x"}}}}"#;
    assert!(matches!(cancel_payload(text), Err(ConvertError::Phases(_))));
}

#[test]
fn request_cancel_payload_rejects_an_async_invoke_status() {
    let text =
        r#"{"async_invoke":{"invocationArn":"arn:aws:batch:1","modelArn":"arn:aws:model:1"}}"#;
    assert!(matches!(cancel_payload(text), Err(ConvertError::Operation)));
}

#[test]
fn request_cancel_input_marks_the_async_call() {
    let call = cancel_payload(CANCEL).unwrap();
    match cancel_input(call, CallMode::Async).call {
        CancelCall::Async { model_in_kwargs } => assert_eq!(model_in_kwargs, None),
        CancelCall::Sync => panic!("the async call must be marked"),
    }
}

#[test]
fn request_cancel_input_keeps_the_model_on_a_sync_call() {
    let call = cancel_payload(CANCEL).unwrap();
    let prepared = cancel_input(call, CallMode::Sync);
    assert_eq!(prepared.model.as_deref(), Some("gpt-4o"));
    assert!(matches!(prepared.call, CancelCall::Sync));
}

#[test]
fn request_hidden_params_report_the_openai_default() {
    let hidden = HiddenParams::new("openai", Some("gpt-4o"), None);
    assert_eq!(hidden.llm_provider, "openai");
    assert_eq!(hidden.model_id, None);
    assert_eq!(hidden.litellm_model_name.as_deref(), Some("gpt-4o"));
    assert_eq!(
        hidden.api_base.as_deref(),
        Some("https://api.openai.com"),
        "the openai default is what get_api_base reports without a caller base"
    );
}

#[test]
fn request_hidden_params_keep_the_given_base() {
    let hidden = HiddenParams::new("openai", Some("gpt-4o"), Some("http://127.0.0.1:1/v1"));
    assert_eq!(hidden.api_base.as_deref(), Some("http://127.0.0.1:1/v1"));
}

#[test]
fn request_hidden_params_report_the_xai_default() {
    let hidden = HiddenParams::new("xai", None, None);
    assert_eq!(hidden.api_base.as_deref(), Some("https://api.x.ai/v1"));
    assert_eq!(hidden.litellm_model_name, None);
}

#[test]
fn request_hidden_params_report_nothing_for_other_providers() {
    let hidden = HiddenParams::new("bedrock", Some("bedrock/model"), None);
    assert_eq!(hidden.api_base, None);
}

#[test]
fn error_list_payload_names_an_unsupported_provider() {
    let error = BatchProviderListError::UnsupportedProvider {
        provider: "anthropic".to_owned(),
    };
    let payload = provider_error_payload(ProviderError::List(&error), "openai", None);
    assert_eq!(payload.error_type, "BadRequestError");
    assert_eq!(payload.status_code, Some(400));
    assert_eq!(payload.llm_provider, "anthropic");
    assert_eq!(payload.model.as_deref(), Some("n/a"));
    assert!(payload.message.contains("anthropic"));
}

#[test]
fn error_list_payload_keeps_the_requested_provider() {
    let error = BatchProviderListError::NonFiniteTimeout;
    let payload = provider_error_payload(ProviderError::List(&error), "openai", None);
    assert_eq!(payload.error_type, "BadRequestError");
    assert_eq!(payload.status_code, Some(400));
    assert_eq!(payload.llm_provider, "openai");
    assert_eq!(payload.model, None);
}

#[test]
fn error_cancel_payload_names_an_unsupported_provider() {
    let error: BatchProviderCancelError<DispatchFailure> = BatchProviderCancelError::Unsupported {
        provider: "anthropic".to_owned(),
    };
    let payload = provider_error_payload(ProviderError::Cancel(&error), "anthropic", None);
    assert_eq!(payload.error_type, "BadRequestError");
    assert_eq!(payload.status_code, Some(400));
    assert_eq!(payload.llm_provider, "anthropic");
    assert_eq!(payload.model.as_deref(), Some("n/a"));
}

#[test]
fn error_cancel_payload_keeps_a_provider_status() {
    let error: BatchProviderCancelError<DispatchFailure> =
        BatchProviderCancelError::Dispatch(DispatchFailure::Status {
            status: 404,
            message: "gone".to_owned(),
            body: None,
        });
    let payload = provider_error_payload(ProviderError::Cancel(&error), "openai", Some("gpt-4o"));
    assert_eq!(payload.error_type, "NotFoundError");
    assert_eq!(payload.status_code, Some(404));
    assert_eq!(payload.message, "gone");
    assert_eq!(payload.llm_provider, "openai");
    assert_eq!(payload.model.as_deref(), Some("gpt-4o"));
}

#[test]
fn error_cancel_payload_keeps_the_provider_body() {
    let error: BatchProviderCancelError<DispatchFailure> =
        BatchProviderCancelError::Dispatch(DispatchFailure::Status {
            status: 400,
            message: "bad cursor".to_owned(),
            body: Some(ErrorBody {
                error_type: Some("invalid_request_error".to_owned()),
                param: Some("after".to_owned()),
                code: None,
            }),
        });
    let payload = provider_error_payload(ProviderError::Cancel(&error), "openai", None);
    let body = payload.body.unwrap();
    assert_eq!(body.error_type.as_deref(), Some("invalid_request_error"));
    assert_eq!(body.param.as_deref(), Some("after"));
    assert_eq!(body.code, None);
}

#[test]
fn error_cancel_payload_reports_a_connection() {
    let error: BatchProviderCancelError<DispatchFailure> =
        BatchProviderCancelError::Dispatch(DispatchFailure::Connection("boom".to_owned()));
    let payload = provider_error_payload(ProviderError::Cancel(&error), "openai", None);
    assert_eq!(payload.error_type, "APIConnectionError");
    assert_eq!(payload.status_code, None);
    assert_eq!(payload.message, "boom");
}

#[test]
fn error_cancel_payload_maps_a_timeout() {
    let error: BatchProviderCancelError<DispatchFailure> =
        BatchProviderCancelError::Timeout(TimeoutError::NonFinite);
    let payload = provider_error_payload(ProviderError::Cancel(&error), "openai", Some("gpt-4o"));
    assert_eq!(payload.error_type, "BadRequestError");
    assert_eq!(payload.status_code, Some(400));
}

#[test]
fn error_cancel_payload_names_the_unwired_branch() {
    let error: BatchProviderCancelError<DispatchFailure> =
        BatchProviderCancelError::Dispatch(DispatchFailure::Unsupported {
            provider: "azure".to_owned(),
        });
    let payload = provider_error_payload(ProviderError::Cancel(&error), "azure", None);
    assert_eq!(payload.error_type, "BadRequestError");
    assert_eq!(payload.status_code, Some(400));
    assert_eq!(payload.llm_provider, "azure");
    assert!(payload.message.contains("azure"));
}

// every status LiteLLM's exception table names has a case; 409 answers the bare APIError here, where the SDK raises ConflictError (docs/deviations/B7873.yaml)
#[rstest]
#[case::bad_request(400, "BadRequestError")]
#[case::authentication(401, "AuthenticationError")]
#[case::permission(403, "PermissionDeniedError")]
#[case::not_found(404, "NotFoundError")]
#[case::conflict(409, "APIError")]
#[case::unprocessable(422, "UnprocessableEntityError")]
#[case::rate_limit(429, "RateLimitError")]
#[case::internal_first(500, "InternalServerError")]
#[case::internal_last(599, "InternalServerError")]
#[case::other(418, "APIError")]
fn error_classes_follow_the_status(#[case] status: u16, #[case] class: &str) {
    assert_eq!(class_for_status(status), class);
}

// the module globals reach the section settings the session resolves once
#[test]
fn request_section_settings_resolve_the_module_values() {
    let module: ModuleSettings =
        serde_json::from_str(r#"{"organization":"org-1","api_base":"http://127.0.0.1:9/v1"}"#)
            .unwrap();
    let settings = section_settings::<batch_provider_list::Raw>(&module, &BTreeMap::new()).unwrap();
    assert_eq!(settings.organization.as_deref(), Some("org-1"));
    assert_eq!(settings.api_base.as_deref(), Some("http://127.0.0.1:9/v1"));
}
