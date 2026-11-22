use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::files::records::{CreatePurpose, ExpiresAfter};
use indexmap::IndexMap;
use rstest::rstest;

use crate::api::file_http_upload::service::Mode;
use crate::api::file_provider_write::{BadRequest, FileWriteError, WriteTimeout};
use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutValue,
};

use super::{Seen, delete, delete_input, sent_data, sent_timeout, upload, upload_input};

const PASSTHROUGH_BODY: &str = "passthrough needs a vertex_ai batch";

fn bad_request<'text>(
    provider: &'text str,
    body: &'static str,
    method: &'static str,
) -> BadRequest<'text> {
    BadRequest {
        model: "n/a",
        llm_provider: provider,
        body,
        method,
        url: "https://github.com/BerriAI/litellm",
    }
}

#[test]
fn upload_optional_fields() {
    let (sent, seen) = upload(Mode::Asynchronous, upload_input(None));
    assert_eq!(sent.unwrap().id, "file-abc");
    let data = sent_data(&seen);
    assert_eq!(data.file.data, b"abc".to_vec());
    assert_eq!(data.file.filename, None);
    assert_eq!(data.purpose, CreatePurpose::Batch);
    assert_eq!(data.expires_after, None);
    assert!(data.extra_headers.is_empty() && data.extra_body.is_empty());
    assert!(matches!(seen.as_slice(), [Seen::OpenAiUpload(..)]));

    let mut detailed = upload_input(Some(Some("openai")));
    detailed.expires_after = Some(ExpiresAfter { seconds: 3600 });
    detailed.extra_headers = Some(IndexMap::from([("X-Trace".to_owned(), "t1".to_owned())]));
    detailed.extra_body = Some(IndexMap::from([("note".to_owned(), "kept".to_owned())]));
    let (_, carried) = upload(Mode::Sync, detailed);
    let request = sent_data(&carried);
    assert_eq!(request.expires_after, Some(ExpiresAfter { seconds: 3600 }));
    assert_eq!(
        request.extra_headers.get("X-Trace").map(String::as_str),
        Some("t1")
    );
    assert_eq!(
        request.extra_body.get("note").map(String::as_str),
        Some("kept")
    );
}

// the undecorated sync function has no default provider, so LlmProviders(None) fails before any request
#[test]
fn sync_upload_without_provider_names_none() {
    let (sent, seen) = upload(Mode::Sync, upload_input(None));
    let error = sent.unwrap_err();
    assert!(matches!(error, FileWriteError::UnknownProvider { .. }));
    assert_eq!(error.to_string(), "None is not a valid LlmProviders");
    assert!(seen.is_empty());
}

// a null provider is no provider: each function's own default applies, as an absent one
#[test]
fn null_provider_takes_the_mode_default() {
    let (sent, seen) = upload(Mode::Asynchronous, upload_input(Some(None)));
    assert_eq!(sent.unwrap().id, "file-abc");
    assert!(matches!(seen.as_slice(), [Seen::OpenAiUpload(..)]));

    let (sync_sent, sync_seen) = upload(Mode::Sync, upload_input(Some(None)));
    assert_eq!(
        sync_sent.unwrap_err().to_string(),
        "None is not a valid LlmProviders"
    );
    assert!(sync_seen.is_empty());
}

#[rstest]
#[case::assistants(CreatePurpose::Assistants)]
#[case::batch(CreatePurpose::Batch)]
#[case::fine_tune(CreatePurpose::FineTune)]
#[case::messages(CreatePurpose::Messages)]
fn upload_forwards_every_purpose(#[case] purpose: CreatePurpose) {
    let mut input = upload_input(None);
    input.purpose = purpose;
    let (_, seen) = upload(Mode::Asynchronous, input);
    assert_eq!(sent_data(&seen).purpose, purpose);
}

#[test]
fn upload_without_logger_fails_first() {
    let mut input = upload_input(Some(Some("openai")));
    input.params.logger = None;
    input.params.passthrough = Some(true);
    let (sent, seen) = upload(Mode::Sync, input);
    let error = sent.unwrap_err();
    assert!(matches!(error, FileWriteError::LoggingObjectRequired));
    assert_eq!(error.to_string(), "logging_obj is required");
    assert_eq!(error.bad_request(), None);
    assert!(seen.is_empty());
}

#[rstest]
#[case::openai_batch(Some("openai"), CreatePurpose::Batch, "openai", "batch")]
#[case::vertex_assistants(
    Some("vertex_ai"),
    CreatePurpose::Assistants,
    "vertex_ai",
    "assistants"
)]
#[case::missing_provider(None, CreatePurpose::Batch, "None", "batch")]
fn upload_passthrough_guard(
    #[case] provider: Option<&str>,
    #[case] purpose: CreatePurpose,
    #[case] named: &str,
    #[case] wire: &str,
) {
    let mut input = upload_input(Some(provider));
    input.purpose = purpose;
    input.params.passthrough = Some(true);
    let (sent, seen) = upload(Mode::Sync, input);
    let error = sent.unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "`passthrough=True` uploads the file bytes unchanged for a native Vertex AI batch, so it needs custom_llm_provider='vertex_ai' and purpose='batch', got '{named}' and '{wire}'."
        )
    );
    let reported = if provider.is_some() { named } else { "n/a" };
    assert_eq!(
        error.bad_request(),
        Some(bad_request(reported, PASSTHROUGH_BODY, "CREATE_FILE"))
    );
    assert!(seen.is_empty());
}

#[rstest]
#[case::vertex_batch(Some(true), "vertex_ai")]
#[case::false_flag(Some(false), "openai")]
#[case::absent_flag(None, "openai")]
fn passthrough_guard_lets_through(#[case] flag: Option<bool>, #[case] provider: &str) {
    let mut input = upload_input(Some(Some(provider)));
    input.params.passthrough = flag;
    let (sent, seen) = upload(Mode::Sync, input);
    assert_eq!(sent.unwrap().id, "file-abc");
    assert_eq!(seen.len(), 1);
}

// only upload validates passthrough; a delete carrying the flag dispatches as usual
#[test]
fn delete_has_no_passthrough_guard() {
    let mut input = delete_input(Some("openai"));
    input.params.passthrough = Some(true);
    let (sent, seen) = delete(Mode::Sync, input);
    assert_eq!(sent.unwrap().id, "file-abc");
    assert!(matches!(seen.as_slice(), [Seen::OpenAiDelete(..)]));
}

fn structured(read: Option<f64>) -> WriteTimeout {
    WriteTimeout::Structured(StructuredTimeout {
        connect: Some(1.0_f64),
        read,
        write: None,
        pool: None,
    })
}

fn number(seconds: f64) -> TimeoutValue {
    TimeoutValue::Number(seconds)
}

fn text(value: &str) -> TimeoutValue {
    TimeoutValue::String(value.to_owned())
}

#[rstest]
#[case::absent(None, None, "openai", FineTuningTimeout::Seconds(600.0_f64))]
#[case::zero_falls_through(
    Some(WriteTimeout::Value(number(0.0_f64))),
    Some(number(3.0_f64)),
    "openai",
    FineTuningTimeout::Seconds(3.0_f64)
)]
#[case::empty_falls_through(
    Some(WriteTimeout::Value(text(""))),
    Some(number(4.0_f64)),
    "openai",
    FineTuningTimeout::Seconds(4.0_f64)
)]
#[case::timeout_wins(
    Some(WriteTimeout::Value(number(2.0_f64))),
    Some(number(3.0_f64)),
    "openai",
    FineTuningTimeout::Seconds(2.0_f64)
)]
#[case::numeric_text(
    Some(WriteTimeout::Value(text("2.5"))),
    None,
    "openai",
    FineTuningTimeout::Seconds(2.5_f64)
)]
#[case::request_text(None, Some(text("7.5")), "openai", FineTuningTimeout::Seconds(7.5_f64))]
#[case::request_zero(
    None,
    Some(number(0.0_f64)),
    "openai",
    FineTuningTimeout::Seconds(600.0_f64)
)]
#[case::structured_openai(Some(structured(Some(5.0_f64))), None, "openai", FineTuningTimeout::Structured(StructuredTimeout { connect: Some(1.0_f64), read: Some(5.0_f64), write: None, pool: None }))]
#[case::structured_azure(Some(structured(Some(5.0_f64))), None, "azure", FineTuningTimeout::Structured(StructuredTimeout { connect: Some(1.0_f64), read: Some(5.0_f64), write: None, pool: None }))]
#[case::structured_anthropic_read(
    Some(structured(Some(5.0_f64))),
    None,
    "anthropic",
    FineTuningTimeout::Seconds(5.0_f64)
)]
#[case::structured_anthropic_default(
    Some(structured(None)),
    None,
    "anthropic",
    FineTuningTimeout::Seconds(600.0_f64)
)]
fn write_timeout_coercion(
    #[case] timeout: Option<WriteTimeout>,
    #[case] request_timeout: Option<TimeoutValue>,
    #[case] provider: &str,
    #[case] expected: FineTuningTimeout,
) {
    let mut uploading = upload_input(Some(Some(provider)));
    uploading.params.timeout = timeout.clone();
    uploading.params.request_timeout = request_timeout.clone();
    let (_, uploaded) = upload(Mode::Asynchronous, uploading);
    assert_eq!(sent_timeout(&uploaded), expected);
    let mut deleting = delete_input(Some(provider));
    deleting.params.timeout = timeout;
    deleting.params.request_timeout = request_timeout;
    let (_, deleted) = delete(Mode::Asynchronous, deleting);
    assert_eq!(sent_timeout(&deleted), expected);
}

#[rstest]
#[case::timeout(Some(WriteTimeout::Value(text("soon"))), None)]
#[case::request_timeout(None, Some(text("soon")))]
fn invalid_timeout_text_fails_before_http(
    #[case] timeout: Option<WriteTimeout>,
    #[case] request_timeout: Option<TimeoutValue>,
) {
    let mut input = upload_input(Some(Some("openai")));
    input.params.timeout = timeout;
    input.params.request_timeout = request_timeout;
    let (sent, seen) = upload(Mode::Sync, input);
    let error = sent.unwrap_err();
    assert!(matches!(error, FileWriteError::TimeoutText { .. }));
    assert_eq!(
        error.to_string(),
        "could not convert string to float: 'soon'"
    );
    assert!(seen.is_empty());
}

// a deviation: LiteLLM's float() accepts inf and NaN, number or text, and hands them to the client
#[rstest]
#[case::number(number(f64::INFINITY))]
#[case::text(text("inf"))]
#[case::nan_text(text("nan"))]
fn infinite_timeout_is_rejected(#[case] timeout: TimeoutValue) {
    let mut input = delete_input(Some("openai"));
    input.params.timeout = Some(WriteTimeout::Value(timeout));
    let (sent, seen) = delete(Mode::Sync, input);
    assert!(matches!(sent.unwrap_err(), FileWriteError::Timeout(_)));
    assert!(seen.is_empty());
}

#[rstest]
#[case::sync(Mode::Sync)]
#[case::asynchronous(Mode::Asynchronous)]
fn write_mode_reaches_the_provider(#[case] mode: Mode) {
    let (_, uploaded) = upload(mode, upload_input(Some(Some("openai"))));
    assert!(matches!(uploaded.as_slice(), [Seen::OpenAiUpload(call, _)] if call.mode == mode));
    let (_, deleted) = delete(mode, delete_input(Some("anthropic")));
    assert!(
        matches!(deleted.as_slice(), [Seen::ConfigDelete { mode: sent, config: ProviderId::Anthropic, .. }] if *sent == mode)
    );
}

#[test]
fn unsupported_write() {
    let (uploaded, upload_calls) = upload(Mode::Sync, upload_input(Some(Some("cohere"))));
    let upload_error = uploaded.unwrap_err();
    assert_eq!(
        upload_error.to_string(),
        "LiteLLM doesn't support cohere for 'create_file'. Only ['openai', 'azure', 'vertex_ai', 'manus', 'anthropic'] are supported."
    );
    assert_eq!(
        upload_error.bad_request(),
        Some(bad_request("cohere", "Unsupported provider", "CREATE_FILE"))
    );
    let (deleted, delete_calls) = delete(Mode::Sync, delete_input(Some("cohere")));
    let delete_error = deleted.unwrap_err();
    assert_eq!(
        delete_error.to_string(),
        "LiteLLM doesn't support cohere for 'file_delete'. Only 'openai', 'azure', 'gemini', 'manus', and 'anthropic' are supported."
    );
    assert_eq!(
        delete_error.bad_request(),
        Some(bad_request(
            "cohere",
            "Unsupported provider",
            "CREATE_THREAD"
        ))
    );
    assert!(upload_calls.is_empty() && delete_calls.is_empty());
}

#[test]
fn unknown_provider_text_fails_conversion() {
    let (uploaded, upload_calls) = upload(
        Mode::Asynchronous,
        upload_input(Some(Some("not-a-provider"))),
    );
    let (deleted, delete_calls) = delete(Mode::Asynchronous, delete_input(Some("not-a-provider")));
    let upload_error = uploaded.unwrap_err();
    let delete_error = deleted.unwrap_err();
    assert_eq!(
        upload_error.to_string(),
        "'not-a-provider' is not a valid LlmProviders"
    );
    assert_eq!(delete_error.to_string(), upload_error.to_string());
    assert_eq!(delete_error.bad_request(), None);
    assert!(upload_calls.is_empty() && delete_calls.is_empty());
}
