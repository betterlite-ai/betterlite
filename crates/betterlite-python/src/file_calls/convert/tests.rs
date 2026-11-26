#![cfg(test)]

use std::time::Duration;

use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_azure::endpoints::initialize::EndpointError;
use betterlite_core::api::file_http_upload::service::Mode;
use betterlite_core::api::file_provider_read::error::{FileReadError, ReadFunction};
use betterlite_core::api::file_provider_read::target::{AzureRead, ReadMode};
use betterlite_core::api::file_provider_read::{Kwarg, ReadProvider};
use betterlite_core::api::file_provider_write::{DeleteParams, FileWriteError, WriteTimeout};
use betterlite_core::api::fine_tuning_read::{ReadTimeout, ReadTimeoutValue};
use betterlite_core::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError, TimeoutValue,
};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::files::error::{FilesError, StatusError};
use betterlite_openai::files::records::CreatePurpose;
use pyo3::prelude::*;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::super::dispatch::DispatchError;
use super::super::dispatch::clients::{
    deadline, delete_litellm_params, is_asynchronous, read_litellm_params, settings,
    write_litellm_params,
};
use super::super::error::{Failure, azure_failure, files_failure, read_failure, write_failure};
use super::super::register;
use super::{ConvertError, parse_create, parse_delete, parse_list, parse_retrieve};

const CREATE: &str = r#"{"file":{"$file":{"name":"x.jsonl","blob":0,"content_type":"application/json"}},"purpose":"batch","expires_after":{"anchor":"created_at","seconds":3600},"custom_llm_provider":"openai","extra_headers":{"x-test":"1"},"extra_body":{"note":"n"},"params":{"api_key":"sk-test","azure_ad_token":"token","tenant_id":"tenant","client_id":"client","client_secret":"secret","azure_username":"user","azure_password":"password","azure_scope":"scope","api_base":"http://127.0.0.1:1/v1","api_version":"2024-01-01","organization":"org-1","timeout":2.5,"request_timeout":"3","max_retries":4,"litellm_call_id":"call-1","id":"fn-1","passthrough":true}}"#;

const DELETE: &str = r#"{"file_id":"file-abc","model":"openai/gpt-4o","custom_llm_provider":"azure","extra_headers":{"x-test":"1"},"extra_body":{"note":"n"},"params":{"api_base":"http://127.0.0.1:1/v1","api_key":"sk-test","timeout":"2.5","max_retries":4}}"#;

const RETRIEVE: &str = r#"{"file_id":"file-abc","custom_llm_provider":"azure","extra_headers":{"x-test":"1"},"extra_body":{"note":"n"},"params":{"azure_ad_token":"token","tenant_id":"tenant","client_id":"client","client_secret":"secret","azure_username":"user","azure_password":"password","azure_scope":"scope","api_base":"http://127.0.0.1:1/v1","api_key":"sk-test","api_version":"2024-01-01","organization":"org-1","timeout":2.5,"request_timeout":"3","max_retries":4,"litellm_call_id":"call-1","id":"fn-1"}}"#;

fn blobs() -> Vec<Vec<u8>> {
    vec![b"payload".to_vec()]
}

#[test]
fn request_register_adds_the_session_and_the_error() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeFileCallSession").unwrap());
        assert!(module.hasattr("NativeFileCallError").unwrap());
    });
}

#[test]
fn request_create_reads_every_field_the_call_takes() {
    let mut blobs = blobs();
    let input = parse_create(CREATE, &mut blobs, Mode::Sync).unwrap();
    assert_eq!(input.file.filename, Some(String::from("x.jsonl")));
    assert_eq!(
        input.file.content_type,
        Some(String::from("application/json"))
    );
    assert_eq!(input.file.data.as_slice(), b"payload");
    assert!(blobs[0].is_empty());
    assert_eq!(input.purpose, CreatePurpose::Batch);
    assert_eq!(
        input.expires_after.map(|expires| expires.seconds),
        Some(3_600)
    );
    assert_eq!(
        input.custom_llm_provider,
        Some(Some(String::from("openai")))
    );
    assert_eq!(
        input
            .extra_headers
            .as_ref()
            .and_then(|headers| headers.get("x-test"))
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        input
            .extra_body
            .as_ref()
            .and_then(|body| body.get("note"))
            .map(String::as_str),
        Some("n")
    );
    assert_eq!(
        input.params.api_base.as_deref(),
        Some("http://127.0.0.1:1/v1")
    );
    assert_eq!(
        input
            .params
            .api_key
            .as_ref()
            .map(SecretString::expose_secret),
        Some("sk-test")
    );
    assert_eq!(input.params.api_version.as_deref(), Some("2024-01-01"));
    assert_eq!(input.params.organization.as_deref(), Some("org-1"));
    assert_eq!(
        input.params.timeout,
        Some(WriteTimeout::Value(TimeoutValue::Number(2.5_f64)))
    );
    assert_eq!(
        input.params.request_timeout,
        Some(TimeoutValue::String(String::from("3")))
    );
    assert_eq!(input.params.max_retries, Some(4_i64));
    assert_eq!(input.params.passthrough, Some(true));
    assert_eq!(
        input.params.litellm_call_id,
        Some(Some(String::from("call-1")))
    );
    assert_eq!(input.params.id.as_deref(), Some("fn-1"));
    assert_eq!(input.params.logger, Some(()));
    let credentials = input.params.credentials.unwrap();
    assert_eq!(credentials.values.len(), 7);
    assert_eq!(credentials.values["tenant_id"].expose_secret(), "tenant");
}

#[test]
fn request_create_with_only_its_required_fields_takes_the_defaults() {
    let mut blobs = blobs();
    let input = parse_create(
        r#"{"file":{"$file":{"blob":0}},"purpose":"batch"}"#,
        &mut blobs,
        Mode::Sync,
    )
    .unwrap();
    assert_eq!(input.file.filename, Some(String::from("file")));
    assert_eq!(input.file.content_type, None);
    assert_eq!(input.file.data.as_slice(), b"payload");
    assert_eq!(input.purpose, CreatePurpose::Batch);
    assert_eq!(input.expires_after, None);
    assert_eq!(input.custom_llm_provider, None);
    assert_eq!(input.extra_headers, None);
    assert_eq!(input.extra_body, None);
    assert_eq!(input.params.api_base, None);
    assert_eq!(input.params.api_key, None);
    assert_eq!(input.params.timeout, None);
    assert_eq!(input.params.request_timeout, None);
    assert_eq!(input.params.max_retries, None);
    assert_eq!(input.params.litellm_call_id, None);
    assert_eq!(input.params.id, None);
    assert_eq!(input.params.passthrough, None);
    let credentials = input.params.credentials.unwrap();
    assert!(credentials.values.is_empty());
}

#[test]
fn request_create_keeps_a_present_null_provider_for_the_sync_default() {
    let mut blobs = blobs();
    let input = parse_create(
        r#"{"file":{"$file":{"blob":0}},"purpose":"batch","custom_llm_provider":null}"#,
        &mut blobs,
        Mode::Sync,
    )
    .unwrap();
    assert_eq!(input.custom_llm_provider, Some(None));
}

#[test]
fn error_create_refuses_a_present_null_provider_in_the_async_mode() {
    let mut blobs = blobs();
    let error = parse_create(
        r#"{"file":{"$file":{"blob":0}},"purpose":"batch","custom_llm_provider":null}"#,
        &mut blobs,
        Mode::Asynchronous,
    )
    .unwrap_err();
    assert!(matches!(error, ConvertError::Provider(_)));
    assert_eq!(error.to_string(), "None is not a valid LlmProviders");
}

#[test]
fn error_create_a_file_that_is_no_marker_is_missing() {
    let mut blobs = blobs();
    let error = parse_create(
        r#"{"file":"plain","purpose":"batch"}"#,
        &mut blobs,
        Mode::Sync,
    )
    .unwrap_err();
    assert!(matches!(error, ConvertError::MissingFile));
}

#[test]
fn error_create_a_foreign_expires_anchor_is_refused() {
    let mut blobs = blobs();
    let error = parse_create(
        r#"{"file":{"$file":{"blob":0}},"purpose":"batch","expires_after":{"anchor":"updated_at","seconds":1}}"#,
        &mut blobs,
        Mode::Sync,
    )
    .unwrap_err();
    assert!(matches!(error, ConvertError::Field("expires_after.anchor")));
}

#[test]
fn error_create_an_unknown_field_is_refused() {
    let mut blobs = blobs();
    let error = parse_create(
        r#"{"file":{"$file":{"blob":0}},"purpose":"batch","nope":1}"#,
        &mut blobs,
        Mode::Sync,
    )
    .unwrap_err();
    assert!(matches!(error, ConvertError::Json(_)));
}

#[test]
fn request_delete_reads_every_field_the_call_takes() {
    let input = parse_delete(DELETE).unwrap();
    assert_eq!(input.file_id, "file-abc");
    assert_eq!(input.model.as_deref(), Some("openai/gpt-4o"));
    assert_eq!(input.custom_llm_provider.as_deref(), Some("azure"));
    assert_eq!(
        input
            .extra_headers
            .as_ref()
            .and_then(|headers| headers.get("x-test"))
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        input
            .extra_body
            .as_ref()
            .and_then(|body| body.get("note"))
            .map(String::as_str),
        Some("n")
    );
    assert_eq!(
        input.params.api_base.as_deref(),
        Some("http://127.0.0.1:1/v1")
    );
    assert_eq!(
        input
            .params
            .api_key
            .as_ref()
            .map(SecretString::expose_secret),
        Some("sk-test")
    );
    assert_eq!(
        input.params.timeout,
        Some(WriteTimeout::Value(TimeoutValue::String(String::from(
            "2.5"
        ))))
    );
    assert_eq!(input.params.max_retries, Some(4_i64));
}

#[test]
fn request_delete_without_a_provider_leaves_the_default_to_the_core() {
    let input = parse_delete(r#"{"file_id":"file-abc"}"#).unwrap();
    assert_eq!(input.file_id, "file-abc");
    assert_eq!(input.model, None);
    assert_eq!(input.custom_llm_provider, None);
}

#[test]
fn error_delete_a_present_null_provider_is_refused() {
    let error = parse_delete(r#"{"file_id":"f","custom_llm_provider":null}"#).unwrap_err();
    assert!(matches!(error, ConvertError::Provider(_)));
}

#[test]
fn request_retrieve_reads_every_field_the_call_takes() {
    let call = parse_retrieve(RETRIEVE).unwrap();
    assert_eq!(call.file_id, "file-abc");
    assert_eq!(
        call.kwargs.custom_llm_provider,
        ReadProvider(String::from("azure"))
    );
    assert_eq!(
        call.kwargs
            .extra_headers
            .as_ref()
            .and_then(|headers| headers.get("x-test"))
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        call.kwargs
            .extra_body
            .as_ref()
            .and_then(|body| body.get("note"))
            .map(String::as_str),
        Some("n")
    );
    assert_eq!(
        call.kwargs.api_base.as_deref(),
        Some("http://127.0.0.1:1/v1")
    );
    assert_eq!(
        call.kwargs
            .api_key
            .as_ref()
            .map(SecretString::expose_secret),
        Some("sk-test")
    );
    assert_eq!(call.kwargs.api_version.as_deref(), Some("2024-01-01"));
    assert_eq!(call.kwargs.organization.as_deref(), Some("org-1"));
    let Some(ReadTimeout::Plain(ReadTimeoutValue::Number(number))) = call.kwargs.timeout else {
        panic!("the timeout is not the number it was given");
    };
    assert_eq!(number.as_f64(), Some(2.5_f64));
    let Some(ReadTimeoutValue::Text(text)) = call.kwargs.request_timeout else {
        panic!("the request timeout is not the text it was given");
    };
    assert_eq!(text, "3");
    assert_eq!(call.kwargs.max_retries, Some(4_i64));
    assert!(!call.kwargs.is_async);
    assert_eq!(
        call.kwargs.litellm_call_id,
        Kwarg::Text(String::from("call-1"))
    );
    assert_eq!(call.kwargs.id, Kwarg::Text(String::from("fn-1")));
    assert_eq!(call.kwargs.after, None);
    assert_eq!(call.kwargs.limit, None);
    assert_eq!(call.kwargs.order, None);
    let values = call.trusted.values;
    assert_eq!(values.len(), 7);
    assert_eq!(values["azure_ad_token"].expose_secret(), "token");
    assert_eq!(values["tenant_id"].expose_secret(), "tenant");
    assert_eq!(values["client_id"].expose_secret(), "client");
    assert_eq!(values["client_secret"].expose_secret(), "secret");
    assert_eq!(values["azure_username"].expose_secret(), "user");
    assert_eq!(values["azure_password"].expose_secret(), "password");
    assert_eq!(values["azure_scope"].expose_secret(), "scope");
}

#[test]
fn request_retrieve_without_a_provider_takes_openai() {
    let call = parse_retrieve(r#"{"file_id":"file-abc"}"#).unwrap();
    assert_eq!(call.kwargs.custom_llm_provider, ReadProvider::default());
    assert_eq!(call.kwargs.litellm_call_id, Kwarg::Absent);
    assert_eq!(call.kwargs.id, Kwarg::Absent);
    assert!(call.trusted.values.is_empty());
}

#[test]
fn error_retrieve_a_present_null_provider_is_refused() {
    let error = parse_retrieve(r#"{"file_id":"f","custom_llm_provider":null}"#).unwrap_err();
    assert!(matches!(error, ConvertError::Provider(_)));
}

#[test]
fn request_list_carries_the_purpose_and_the_provider() {
    let call = parse_list(r#"{"purpose":"batch","custom_llm_provider":"manus"}"#).unwrap();
    assert_eq!(call.purpose.as_deref(), Some("batch"));
    assert_eq!(
        call.kwargs.custom_llm_provider,
        ReadProvider(String::from("manus"))
    );
}

#[test]
fn request_list_without_a_purpose_or_provider_takes_the_defaults() {
    let call = parse_list("{}").unwrap();
    assert_eq!(call.purpose, None);
    assert_eq!(call.kwargs.custom_llm_provider, ReadProvider::default());
}

#[test]
fn error_list_a_present_null_provider_is_refused() {
    let error = parse_list(r#"{"custom_llm_provider":null}"#).unwrap_err();
    assert!(matches!(error, ConvertError::Provider(_)));
}

#[test]
fn request_read_structured_timeout_becomes_the_phases() {
    let call = parse_retrieve(
        r#"{"file_id":"f","params":{"timeout":{"connect":1.5,"read":2,"write":3,"pool":4}}}"#,
    )
    .unwrap();
    let Some(ReadTimeout::Structured(phases)) = call.kwargs.timeout else {
        panic!("the timeout is not the phases it was given");
    };
    assert_eq!(
        phases.connect.and_then(|number| number.as_f64()),
        Some(1.5_f64)
    );
    assert_eq!(
        phases.read.and_then(|number| number.as_f64()),
        Some(2.0_f64)
    );
    assert_eq!(
        phases.write.and_then(|number| number.as_f64()),
        Some(3.0_f64)
    );
    assert_eq!(
        phases.pool.and_then(|number| number.as_f64()),
        Some(4.0_f64)
    );
}

#[test]
fn request_read_a_text_timeout_stays_text() {
    let call = parse_retrieve(r#"{"file_id":"f","params":{"timeout":"1.5"}}"#).unwrap();
    let Some(ReadTimeout::Plain(ReadTimeoutValue::Text(text))) = call.kwargs.timeout else {
        panic!("the timeout is not the text it was given");
    };
    assert_eq!(text, "1.5");
}

#[test]
fn request_write_structured_timeout_becomes_the_phases() {
    let input =
        parse_delete(r#"{"file_id":"f","params":{"timeout":{"connect":1,"read":2}}}"#).unwrap();
    let Some(WriteTimeout::Structured(phases)) = input.params.timeout else {
        panic!("the timeout is not the phases it was given");
    };
    assert_eq!(phases.connect, Some(1.0_f64));
    assert_eq!(phases.read, Some(2.0_f64));
    assert_eq!(phases.write, None);
    assert_eq!(phases.pool, None);
}

#[test]
fn error_a_timeout_shape_the_create_does_not_take_is_refused() {
    let mut blobs = blobs();
    let error = parse_create(
        r#"{"file":{"$file":{"blob":0}},"purpose":"batch","params":{"timeout":true}}"#,
        &mut blobs,
        Mode::Sync,
    )
    .unwrap_err();
    assert!(matches!(error, ConvertError::TimeoutShapes(_)));
}

#[test]
fn error_a_request_timeout_that_carries_the_phases_is_refused() {
    let error =
        parse_delete(r#"{"file_id":"f","params":{"request_timeout":{"read":1}}}"#).unwrap_err();
    assert!(matches!(error, ConvertError::TimeoutShapes(_)));
}

#[test]
fn error_a_retrieve_request_timeout_that_is_no_value_is_refused() {
    let error = parse_retrieve(r#"{"file_id":"f","params":{"request_timeout":true}}"#).unwrap_err();
    assert!(matches!(error, ConvertError::TimeoutShapes(_)));
}

#[test]
fn error_retrieve_an_unknown_field_is_refused() {
    let error = parse_retrieve(r#"{"file_id":"f","nope":1}"#).unwrap_err();
    assert!(matches!(error, ConvertError::Json(_)));
}

#[test]
fn request_azure_upload_params_carry_the_trusted_credentials() {
    let mut blobs = blobs();
    let input = parse_create(CREATE, &mut blobs, Mode::Sync).unwrap();
    let resolved = StructuredTimeout {
        connect: Some(1.0_f64),
        read: Some(7.0_f64),
        write: None,
        pool: None,
    };
    let params = write_litellm_params(&input.params, FineTuningTimeout::Structured(resolved));
    assert_eq!(
        params
            .azure_ad_token
            .as_ref()
            .map(SecretString::expose_secret),
        Some("token")
    );
    assert_eq!(params.auth.tenant_id.as_deref(), Some("tenant"));
    assert_eq!(params.auth.client_id.as_deref(), Some("client"));
    assert_eq!(
        params
            .auth
            .client_secret
            .as_ref()
            .map(SecretString::expose_secret),
        Some("secret")
    );
    assert_eq!(params.auth.azure_username.as_deref(), Some("user"));
    assert_eq!(
        params
            .auth
            .azure_password
            .as_ref()
            .map(SecretString::expose_secret),
        Some("password")
    );
    assert_eq!(params.auth.azure_scope.as_deref(), Some("scope"));
    assert_eq!(params.max_retries, Some(4_i64));
    assert_eq!(params.timeout, Some(7.0_f64));
}

#[test]
fn request_azure_delete_params_carry_the_resolved_timeout_and_the_credentials() {
    let mut blobs = blobs();
    let create = parse_create(CREATE, &mut blobs, Mode::Sync).unwrap();
    let credentials = create.params.credentials.as_ref().unwrap();
    let delete = DeleteParams {
        api_key: None,
        api_base: None,
        credentials: Some(credentials),
    };
    let params = delete_litellm_params(&delete, FineTuningTimeout::Seconds(2.5_f64), Some(4_i64));
    assert_eq!(params.timeout, Some(2.5_f64));
    assert_eq!(params.max_retries, Some(4_i64));
    assert_eq!(
        params
            .azure_ad_token
            .as_ref()
            .map(SecretString::expose_secret),
        Some("token")
    );
    assert_eq!(params.auth.tenant_id.as_deref(), Some("tenant"));
    let phases = StructuredTimeout {
        connect: Some(1.0_f64),
        read: Some(2.0_f64),
        write: None,
        pool: None,
    };
    let structured = delete_litellm_params(&delete, FineTuningTimeout::Structured(phases), None);
    assert_eq!(structured.timeout, Some(2.0_f64));
    assert_eq!(structured.max_retries, None);
    let absent = DeleteParams {
        api_key: None,
        api_base: None,
        credentials: None,
    };
    let empty = delete_litellm_params(
        &absent,
        FineTuningTimeout::Structured(StructuredTimeout {
            connect: None,
            read: None,
            write: None,
            pool: None,
        }),
        None,
    );
    assert_eq!(empty.timeout, Some(600.0_f64));
    assert_eq!(empty.azure_ad_token, None);
    assert_eq!(empty.auth.tenant_id, None);
}

fn azure_read(timeout: FineTuningTimeout) -> AzureRead {
    AzureRead {
        api_base: Some(String::from("https://example.openai.azure.com")),
        api_key: Some(SecretString::from("sk-test")),
        api_version: Some(String::from("2024-01-01")),
        max_retries: None,
        mode: ReadMode {
            is_async: false,
            timeout,
        },
    }
}

#[test]
fn request_azure_read_params_carry_the_resolved_timeout_and_retries() {
    let all_phases = StructuredTimeout {
        connect: Some(1.0_f64),
        read: Some(2.0_f64),
        write: Some(3.0_f64),
        pool: Some(4.0_f64),
    };
    let params = read_litellm_params(&azure_read(FineTuningTimeout::Structured(all_phases)));
    assert_eq!(params.timeout, Some(2.0_f64));
    assert_eq!(params.azure_ad_token, None);
    assert_eq!(params.auth.tenant_id, None);
    assert_eq!(params.auth.client_secret, None);
    let connect_only = StructuredTimeout {
        connect: Some(1.0_f64),
        read: None,
        write: None,
        pool: None,
    };
    assert_eq!(
        read_litellm_params(&azure_read(FineTuningTimeout::Structured(connect_only))).timeout,
        Some(1.0_f64)
    );
    let write_only = StructuredTimeout {
        connect: None,
        read: None,
        write: Some(3.0_f64),
        pool: Some(4.0_f64),
    };
    assert_eq!(
        read_litellm_params(&azure_read(FineTuningTimeout::Structured(write_only))).timeout,
        Some(3.0_f64)
    );
    let pool_only = StructuredTimeout {
        connect: None,
        read: None,
        write: None,
        pool: Some(4.0_f64),
    };
    assert_eq!(
        read_litellm_params(&azure_read(FineTuningTimeout::Structured(pool_only))).timeout,
        Some(4.0_f64)
    );
    let seconds = read_litellm_params(&azure_read(FineTuningTimeout::Seconds(2.5_f64)));
    assert_eq!(seconds.timeout, Some(2.5_f64));
    let retried = read_litellm_params(&AzureRead {
        max_retries: Some(3_i64),
        ..azure_read(FineTuningTimeout::Seconds(1.0_f64))
    });
    assert_eq!(retried.max_retries, Some(3_i64));
    assert_eq!(retried.timeout, Some(1.0_f64));
}

#[test]
fn request_the_client_settings_are_the_handlers_own_arguments() {
    let built = settings(
        Some(SecretString::from("sk-test")),
        Some(String::from("http://127.0.0.1:1/v1")),
        Some(String::from("2024-01-01")),
    );
    assert_eq!(
        built.api_key.as_ref().map(SecretString::expose_secret),
        Some("sk-test")
    );
    assert_eq!(built.api_base.as_deref(), Some("http://127.0.0.1:1/v1"));
    assert_eq!(built.api_version.as_deref(), Some("2024-01-01"));
}

#[test]
fn request_the_client_flag_is_the_call_mode() {
    assert!(!is_asynchronous(Mode::Sync));
    assert!(is_asynchronous(Mode::Asynchronous));
}

#[rstest]
#[case::read(r#"{"read":0.05,"connect":1,"write":2,"pool":3}"#, 0.05_f64)]
#[case::connect(r#"{"connect":1,"write":2,"pool":3}"#, 1.0_f64)]
#[case::write(r#"{"write":2,"pool":3}"#, 2.0_f64)]
#[case::pool(r#"{"pool":3}"#, 3.0_f64)]
#[case::empty("{}", 600.0_f64)]
fn request_phase_deadline(#[case] text: &str, #[case] expected: f64) {
    let phases: StructuredTimeout = serde_json::from_str(text).unwrap();
    assert_eq!(
        deadline(FineTuningTimeout::Structured(phases)),
        Some(Duration::try_from_secs_f64(expected).unwrap())
    );
    assert_eq!(
        deadline(FineTuningTimeout::Seconds(expected)),
        Some(Duration::try_from_secs_f64(expected).unwrap())
    );
}

#[test]
fn error_a_status_is_the_class_its_code_names() {
    let status = StatusError {
        status: 404_u16,
        message: String::from("Error code: 404 - missing"),
        error_type: Some(String::from("invalid_request_error")),
        param: None,
        code: Some(String::from("file_not_found")),
    };
    let failure = files_failure(&FilesError::Status(Box::new(status)));
    let Failure::Payload(payload) = failure else {
        panic!("the status error did not become a payload");
    };
    assert_eq!(payload.class, "NotFoundError");
    assert_eq!(payload.status_code, Some(404_u16));
    assert_eq!(payload.message, "Error code: 404 - missing");
    let Some(JsonValue::Object(body)) = payload.body else {
        panic!("the status body is not an object");
    };
    assert_eq!(
        body.get("code"),
        Some(&JsonValue::Text(String::from("file_not_found")))
    );
    assert_eq!(body.get("param"), None);
}

#[test]
fn error_a_status_without_fields_carries_no_body() {
    let status = StatusError {
        status: 400_u16,
        message: String::from("Error code: 400"),
        error_type: None,
        param: None,
        code: None,
    };
    let Failure::Payload(payload) = files_failure(&FilesError::Status(Box::new(status))) else {
        panic!("the status error did not become a payload");
    };
    assert_eq!(payload.class, "BadRequestError");
    assert_eq!(payload.body, None);
}

#[rstest]
#[case::bad_request(400_u16, "BadRequestError")]
#[case::authentication(401_u16, "AuthenticationError")]
#[case::permission_denied(403_u16, "PermissionDeniedError")]
#[case::not_found(404_u16, "NotFoundError")]
#[case::unprocessable_entity(422_u16, "UnprocessableEntityError")]
#[case::rate_limit(429_u16, "RateLimitError")]
#[case::internal_server(500_u16, "InternalServerError")]
#[case::internal_server_at_the_top(599_u16, "InternalServerError")]
#[case::any_other_code(418_u16, "APIError")]
fn error_each_status_class_follows_its_code(#[case] status_code: u16, #[case] class: &str) {
    let status = StatusError {
        status: status_code,
        message: String::new(),
        error_type: None,
        param: None,
        code: None,
    };
    let Failure::Payload(payload) = files_failure(&FilesError::Status(Box::new(status))) else {
        panic!("the status error did not become a payload");
    };
    assert_eq!(payload.class, class);
}

#[test]
fn error_a_status_with_only_a_type_keeps_the_body() {
    let status = StatusError {
        status: 404_u16,
        message: String::from("Error code: 404 - missing"),
        error_type: Some(String::from("invalid_request_error")),
        param: None,
        code: None,
    };
    let Failure::Payload(payload) = files_failure(&FilesError::Status(Box::new(status))) else {
        panic!("the status error did not become a payload");
    };
    let Some(JsonValue::Object(body)) = payload.body else {
        panic!("the status body is not an object");
    };
    assert_eq!(
        body.get("type"),
        Some(&JsonValue::Text(String::from("invalid_request_error")))
    );
    assert_eq!(body.get("code"), None);
}

#[test]
fn error_a_status_with_only_a_code_keeps_the_body() {
    let status = StatusError {
        status: 429_u16,
        message: String::from("Error code: 429 - slow down"),
        error_type: None,
        param: None,
        code: Some(String::from("rate_limited")),
    };
    let Failure::Payload(payload) = files_failure(&FilesError::Status(Box::new(status))) else {
        panic!("the status error did not become a payload");
    };
    let Some(JsonValue::Object(body)) = payload.body else {
        panic!("the status body is not an object");
    };
    assert_eq!(
        body.get("code"),
        Some(&JsonValue::Text(String::from("rate_limited")))
    );
    assert_eq!(body.get("type"), None);
}

#[test]
fn error_an_azure_endpoint_key_failure_is_a_key_error() {
    let failure = azure_failure(&AzureFilesError::Endpoint {
        source: EndpointError::MissingApiVersion,
    });
    assert_eq!(
        failure,
        Failure::Key(String::from("the client custom query has no api-version"))
    );
}

#[test]
fn error_an_azure_missing_credentials_is_a_value_error() {
    let failure = azure_failure(&AzureFilesError::MissingCredentials);
    let Failure::Value(message) = failure else {
        panic!("the missing credentials did not become a value error");
    };
    assert!(message.contains("Missing credentials"));
}

#[test]
fn error_an_azure_token_provider_failure_is_a_runtime_error() {
    let failure = azure_failure(&AzureFilesError::TokenProvider {
        class: String::from("RuntimeError"),
        message: String::from("the token provider broke"),
    });
    assert_eq!(
        failure,
        Failure::Runtime(String::from("the token provider broke"))
    );
}

#[test]
fn error_an_uninitialized_client_is_a_value_error() {
    let failure = files_failure(&FilesError::NotInitialized);
    let Failure::Value(message) = failure else {
        panic!("the uninitialized client did not become a value error");
    };
    assert!(message.contains("not initialized"));
}

#[test]
fn error_a_passthrough_write_is_a_bad_request_naming_the_provider() {
    let error = FileWriteError::<DispatchError>::Passthrough {
        provider: None,
        purpose: "batch",
    };
    let Failure::Payload(payload) = write_failure(&error) else {
        panic!("the passthrough guard did not become a payload");
    };
    assert_eq!(payload.class, "BadRequestError");
    assert_eq!(payload.status_code, Some(400_u16));
    assert_eq!(payload.llm_provider.as_deref(), Some("n/a"));
    assert_eq!(payload.model.as_deref(), Some("n/a"));
    assert_eq!(
        payload.body,
        Some(JsonValue::Text(String::from(
            "passthrough needs a vertex_ai batch"
        )))
    );
}

#[test]
fn error_an_unsupported_upload_is_a_bad_request_naming_the_provider() {
    let error = FileWriteError::<DispatchError>::UnsupportedUpload {
        provider: String::from("cohere"),
    };
    let Failure::Payload(payload) = write_failure(&error) else {
        panic!("the unsupported upload did not become a payload");
    };
    assert_eq!(payload.class, "BadRequestError");
    assert_eq!(payload.llm_provider.as_deref(), Some("cohere"));
    assert!(payload.message.contains("create_file"));
}

#[test]
fn error_an_unsupported_delete_is_a_bad_request_naming_the_provider() {
    let error = FileWriteError::<DispatchError>::UnsupportedDelete {
        provider: String::from("cohere"),
    };
    let Failure::Payload(payload) = write_failure(&error) else {
        panic!("the unsupported delete did not become a payload");
    };
    assert_eq!(payload.class, "BadRequestError");
    assert_eq!(payload.llm_provider.as_deref(), Some("cohere"));
    assert!(payload.message.contains("file_delete"));
}

#[test]
fn error_an_unknown_write_provider_is_a_value_error() {
    let error = FileWriteError::<DispatchError>::UnknownProvider {
        provider: String::from("None"),
    };
    let Failure::Value(message) = write_failure(&error) else {
        panic!("the unknown provider did not become a value error");
    };
    assert!(message.contains("is not a valid LlmProviders"));
}

#[test]
fn error_a_timeout_text_is_a_value_error() {
    let error = FileWriteError::<DispatchError>::TimeoutText {
        text: String::from("soon"),
        source: TimeoutError::NonFinite,
    };
    let Failure::Value(message) = write_failure(&error) else {
        panic!("the timeout text did not become a value error");
    };
    assert!(message.contains("could not convert string to float: 'soon'"));
}

#[test]
fn error_a_read_timeout_is_a_value_error() {
    let error = FileReadError::<DispatchError>::Timeout(TimeoutError::NonFinite);
    let Failure::Value(message) = read_failure(&error) else {
        panic!("the read timeout did not become a value error");
    };
    assert!(message.contains("timeout must be finite"));
}

#[test]
fn error_an_unsupported_read_is_the_bad_request_with_the_placeholder_body() {
    let error = FileReadError::<DispatchError>::Unsupported {
        provider: String::from("cohere"),
        function: ReadFunction::FileRetrieve,
    };
    let Failure::Payload(payload) = read_failure(&error) else {
        panic!("the rejected provider did not become a payload");
    };
    assert_eq!(payload.class, "BadRequestError");
    assert_eq!(payload.status_code, Some(400_u16));
    assert_eq!(payload.llm_provider.as_deref(), Some("cohere"));
    assert_eq!(payload.model.as_deref(), Some("n/a"));
    assert_eq!(
        payload.body,
        Some(JsonValue::Text(String::from("Unsupported provider")))
    );
    assert!(payload.message.contains("file_retrieve"));
    assert!(payload.message.contains("Only 'openai', 'azure'"));
}

#[test]
fn error_a_read_dispatch_reply_is_a_runtime_error() {
    let error = FileReadError::<DispatchError>::Dispatch(DispatchError::Reply);
    assert_eq!(
        read_failure(&error),
        Failure::Runtime(String::from(
            "the file call answered with another reply kind"
        ))
    );
}

#[test]
fn error_a_read_dispatch_for_an_unbuilt_provider_is_not_implemented() {
    let error = FileReadError::<DispatchError>::Dispatch(DispatchError::Unbuilt {
        config: "AnthropicFilesConfig",
    });
    assert_eq!(
        read_failure(&error),
        Failure::Unsupported(String::from(
            "the AnthropicFilesConfig file call is not implemented in this build"
        ))
    );
}

#[test]
fn error_a_read_dispatch_through_files_keeps_the_status_class() {
    let status = StatusError {
        status: 404_u16,
        message: String::from("Error code: 404 - missing"),
        error_type: None,
        param: None,
        code: None,
    };
    let error = FileReadError::<DispatchError>::Dispatch(DispatchError::Files(FilesError::Status(
        Box::new(status),
    )));
    let Failure::Payload(payload) = read_failure(&error) else {
        panic!("the dispatch failure did not become a payload");
    };
    assert_eq!(payload.class, "NotFoundError");
}
