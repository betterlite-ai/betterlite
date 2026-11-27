#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::{batch_provider_create, batch_provider_read, xai_identity};
use betterlite_core::api::api_boundaries::{ApiBoundariesInput, ApiLimits, JsonBoundary};
use betterlite_core::api::batch_provider_create::error::BatchProviderCreateError;
use betterlite_core::api::batch_provider_create::timeout::{HttpTimeout, RequestTimeout};
use betterlite_core::api::batch_provider_create::{BatchProvider, BatchProviderCreateInput};
use betterlite_core::api::batch_provider_read::error::BatchProviderReadError;
use betterlite_core::api::batch_provider_read::{
    BatchProviderReadInput, BatchReadProvider, BatchReadTimeoutValue,
};
use betterlite_core::api::resource_sdk_settings::timeout::TimeoutError;
use betterlite_openai::batches::records::{
    BatchEndpoint, BatchObject, CompletionWindow, ExpiryAnchor, MissingBatchField, OutputExpiry,
};
use betterlite_openai::batches::sdk::TimeoutError as SdkTimeoutError;
use betterlite_openai::batches::sdk::TimeoutPhases;
use secrecy::ExposeSecret as _;
use serde::Deserialize;

use super::{
    BatchAnswer, CallMode, HiddenParams, ModuleSettings, SessionWire, create_input, create_payload,
    parse, read_input, read_payload, section_settings,
};
use crate::batch_calls::error::DispatchFailure;
use crate::batch_calls::payload::{
    ErrorPayload, class_for_status, create_error_payload, dispatch_fields, read_error_payload,
};

const CREATE_PAYLOAD: &str = r#"{
    "completion_window": "24h",
    "endpoint": "/v1/chat/completions",
    "input_file_id": "file-1",
    "custom_llm_provider": "openai",
    "metadata": {"team": "x"},
    "extra_headers": {"x-header": "one"},
    "extra_body": {"key": "value"},
    "output_expires_after": {"anchor": "created_at", "seconds": 3600},
    "model": "gpt-4o",
    "optional_params": {
        "api_base": "http://local/v1",
        "api_key": "sk-test",
        "timeout": {"connect": 1.0, "read": 30.0, "write": 2.0, "pool": 3.0},
        "max_retries": 2,
        "organization": "org-1",
        "vertex_project": "project",
        "vertex_location": "us-east1",
        "custom_endpoint": true
    }
}"#;

const BATCH_JSON: &str = r#"{"id":"batch-1","completion_window":"24h","created_at":0,"endpoint":"/v1/chat/completions","input_file_id":"file-1","object":"batch","status":"validating"}"#;

#[derive(Debug, Deserialize)]
struct AnswerView {
    id: String,
    #[serde(rename = "_hidden_params")]
    hidden_params: HiddenParams,
}

fn read_call(provider: &str, model: Option<&str>) -> BatchProviderReadInput {
    let mut input: BatchProviderReadInput = read_payload(r#"{"batch_id":"batch-1"}"#).unwrap();
    input.provider = BatchReadProvider::from(provider.to_owned());
    input.model = model.map(str::to_owned);
    input
}

#[test]
fn request_create_mode_sets_is_async() {
    let input: BatchProviderCreateInput = create_payload(CREATE_PAYLOAD).unwrap();
    assert!(!create_input(input.clone(), CallMode::Sync).is_async);
    assert!(create_input(input, CallMode::Async).is_async);
}

#[test]
fn request_read_mode_sets_aretrieve_batch() {
    let input: BatchProviderReadInput = read_payload(r#"{"batch_id":"batch-1"}"#).unwrap();
    assert!(!read_input(input.clone(), CallMode::Sync).aretrieve_batch);
    assert!(read_input(input, CallMode::Async).aretrieve_batch);
}

#[test]
fn request_create_payload_keeps_the_identifying_fields() {
    let input: BatchProviderCreateInput = create_payload(CREATE_PAYLOAD).unwrap();
    assert_eq!(input.completion_window, CompletionWindow::Day);
    assert_eq!(input.endpoint, BatchEndpoint::ChatCompletions);
    assert_eq!(input.input_file_id, "file-1");
    assert_eq!(input.custom_llm_provider, BatchProvider::Openai);
    assert_eq!(input.model.as_deref(), Some("gpt-4o"));
}

#[test]
fn request_create_payload_keeps_the_passthrough_maps() {
    let input: BatchProviderCreateInput = create_payload(CREATE_PAYLOAD).unwrap();
    assert_eq!(
        input
            .metadata
            .as_ref()
            .unwrap()
            .get("team")
            .map(String::as_str),
        Some("x")
    );
    assert_eq!(
        input
            .extra_headers
            .as_ref()
            .unwrap()
            .get("x-header")
            .map(String::as_str),
        Some("one")
    );
    assert_eq!(
        input
            .extra_body
            .as_ref()
            .unwrap()
            .get("key")
            .map(String::as_str),
        Some("value")
    );
    assert_eq!(
        input.output_expires_after,
        Some(OutputExpiry {
            anchor: ExpiryAnchor::CreatedAt,
            seconds: 3600,
        })
    );
}

#[test]
fn request_create_payload_keeps_the_optional_params() {
    let input: BatchProviderCreateInput = create_payload(CREATE_PAYLOAD).unwrap();
    assert_eq!(
        input.optional_params.api_base.as_deref(),
        Some("http://local/v1")
    );
    assert_eq!(
        input
            .optional_params
            .api_key
            .as_ref()
            .unwrap()
            .expose_secret(),
        "sk-test"
    );
    assert_eq!(
        input.optional_params.timeout,
        Some(RequestTimeout::Client(HttpTimeout {
            connect: Some(1.0_f64),
            read: Some(30.0_f64),
            write: Some(2.0_f64),
            pool: Some(3.0_f64),
        }))
    );
    assert_eq!(input.optional_params.max_retries, Some(2));
    assert_eq!(input.optional_params.organization.as_deref(), Some("org-1"));
    assert_eq!(
        input.optional_params.vertex_project.as_deref(),
        Some("project")
    );
    assert_eq!(
        input.optional_params.vertex_location.as_deref(),
        Some("us-east1")
    );
    assert_eq!(input.optional_params.custom_endpoint, Some(true));
}

// The timeout crosses the payload as a number or as phases, which the typed parse alone cannot read.
#[test]
fn request_create_payload_reads_a_numeric_timeout() {
    let input: BatchProviderCreateInput = create_payload(
        r#"{"completion_window":"24h","endpoint":"/v1/chat/completions","input_file_id":"f","optional_params":{"timeout":30}}"#,
    )
    .unwrap();
    assert_eq!(
        input.optional_params.timeout,
        Some(RequestTimeout::Seconds(30.0_f64))
    );
}

#[test]
fn request_create_payload_treats_a_null_timeout_as_absent() {
    let input: BatchProviderCreateInput = create_payload(
        r#"{"completion_window":"24h","endpoint":"/v1/chat/completions","input_file_id":"f","optional_params":{"timeout":null,"request_timeout":5}}"#,
    )
    .unwrap();
    assert_eq!(input.optional_params.timeout, None);
    assert_eq!(input.optional_params.request_timeout, Some(5.0_f64));
}

#[test]
fn request_create_payload_refuses_a_bad_timeout() {
    let text = r#"{"completion_window":"24h","endpoint":"/v1/chat/completions","input_file_id":"f","optional_params":{"timeout":"10"}}"#;
    let error = create_payload(text).unwrap_err();
    assert!(error.to_string().contains("optional_params.timeout"));
    let unknown = r#"{"completion_window":"24h","endpoint":"/v1/chat/completions","input_file_id":"f","optional_params":{"timeout":{"connect":1,"deadline":2}}}"#;
    let unknown_error = create_payload(unknown).unwrap_err();
    assert!(unknown_error.to_string().contains("deadline"));
    let wrong = r#"{"completion_window":"24h","endpoint":"/v1/chat/completions","input_file_id":"f","optional_params":{"timeout":{"read":"30"}}}"#;
    let wrong_error = create_payload(wrong).unwrap_err();
    assert!(wrong_error.to_string().contains("expected f64"));
    let huge = r#"{"completion_window":"24h","endpoint":"/v1/chat/completions","input_file_id":"f","optional_params":{"timeout":1e999}}"#;
    let huge_error = create_payload(huge).unwrap_err();
    // serde_json stores the exponent with a sign, so the message names 1e+999
    assert!(huge_error.to_string().contains("1e+999"));
}

#[test]
fn request_create_payload_refuses_an_unknown_key() {
    let text = r#"{"completion_window":"24h","endpoint":"/v1/chat/completions","input_file_id":"f","acreate_batch":true}"#;
    let error = create_payload(text).unwrap_err();
    assert!(error.to_string().contains("acreate_batch"));
    let nested = r#"{"completion_window":"24h","endpoint":"/v1/chat/completions","input_file_id":"f","optional_params":{"num_retries":1}}"#;
    let nested_error = create_payload(nested).unwrap_err();
    assert!(nested_error.to_string().contains("num_retries"));
}

// B1036's request types carry the literals as enums, so the parse, not the provider, refuses a value outside them.
#[test]
fn request_create_payload_refuses_a_value_outside_the_literal() {
    let window =
        r#"{"completion_window":"48h","endpoint":"/v1/chat/completions","input_file_id":"f"}"#;
    let window_error = create_payload(window).unwrap_err();
    assert!(window_error.to_string().contains("48h"));
    let endpoint = r#"{"completion_window":"24h","endpoint":"/v1/files","input_file_id":"f"}"#;
    let endpoint_error = create_payload(endpoint).unwrap_err();
    assert!(endpoint_error.to_string().contains("/v1/files"));
    let provider = r#"{"completion_window":"24h","endpoint":"/v1/chat/completions","input_file_id":"f","custom_llm_provider":"cohere"}"#;
    let provider_error = create_payload(provider).unwrap_err();
    assert!(provider_error.to_string().contains("cohere"));
}

#[test]
fn request_read_payload_defaults_the_provider() {
    let input: BatchProviderReadInput =
        read_payload(r#"{"batch_id":"batch-1","provider":"openai"}"#).unwrap();
    assert_eq!(input.provider, BatchReadProvider::OpenAi);
    assert!(!input.aretrieve_batch);
    assert!(input.model.is_none());
    assert!(input.optional_params.api_base.is_none());
    let defaulted: BatchProviderReadInput = read_payload(r#"{"batch_id":"batch-1"}"#).unwrap();
    assert_eq!(defaulted.provider, BatchReadProvider::OpenAi);
}

#[test]
fn request_read_payload_keeps_an_unknown_provider() {
    let input = read_call("cohere", None);
    assert_eq!(
        input.provider,
        BatchReadProvider::Other("cohere".to_owned())
    );
    assert_eq!(input.provider.name(), "cohere");
    assert!(!input.provider.supports_httpx_timeout());
}

// the read path hands the timeout to the provider wrappers as the value Python gave, so each shape survives
#[test]
fn request_read_payload_reads_each_timeout_shape() {
    let seconds: BatchProviderReadInput =
        read_payload(r#"{"batch_id":"b","optional_params":{"timeout":30}}"#).unwrap();
    assert_eq!(
        seconds.optional_params.timeout,
        Some(BatchReadTimeoutValue::Seconds(30.0_f64))
    );
    let text: BatchProviderReadInput =
        read_payload(r#"{"batch_id":"b","optional_params":{"timeout":"30"}}"#).unwrap();
    assert_eq!(
        text.optional_params.timeout,
        Some(BatchReadTimeoutValue::Text(String::from("30")))
    );
    let phases_input: BatchProviderReadInput =
        read_payload(r#"{"batch_id":"b","optional_params":{"timeout":{"connect":1,"read":2.5}}}"#)
            .unwrap();
    assert_eq!(
        phases_input.optional_params.timeout,
        Some(BatchReadTimeoutValue::Phases(TimeoutPhases {
            connect: Some(1.0_f64),
            read: Some(2.5_f64),
            write: None,
            pool: None,
        }))
    );
    let absent: BatchProviderReadInput =
        read_payload(r#"{"batch_id":"b","optional_params":{"timeout":null}}"#).unwrap();
    assert_eq!(absent.optional_params.timeout, None);
}

#[test]
fn request_read_payload_refuses_a_bad_timeout() {
    let array = r#"{"batch_id":"b","optional_params":{"timeout":[30]}}"#;
    let array_error = read_payload(array).unwrap_err();
    assert!(array_error.to_string().contains("optional_params.timeout"));
    let unknown = r#"{"batch_id":"b","optional_params":{"timeout":{"connect":1,"total":2}}}"#;
    let unknown_error = read_payload(unknown).unwrap_err();
    assert!(unknown_error.to_string().contains("total"));
}

#[test]
fn request_session_wire_reads_the_environment_and_settings() {
    let wire: SessionWire = parse(
        r#"{"environment":{"OPENAI_API_KEY":"sk-env"},"settings":{"api_base":"http://module/v1"}}"#,
    )
    .unwrap();
    assert_eq!(
        wire.environment
            .as_ref()
            .unwrap()
            .get("OPENAI_API_KEY")
            .map(String::as_str),
        Some("sk-env")
    );
    assert_eq!(wire.settings.api_base.as_deref(), Some("http://module/v1"));
    let defaulted: SessionWire = parse("{}").unwrap();
    assert!(defaulted.environment.is_none());
    assert!(defaulted.settings.api_key.is_none());
}

#[test]
fn error_session_wire_refuses_an_unknown_setting() {
    let error = parse::<SessionWire>(r#"{"settings":{"num_retries":1}}"#).unwrap_err();
    assert!(error.to_string().contains("num_retries"));
}

#[test]
fn request_create_settings_resolve_the_environment() {
    let module: ModuleSettings = parse(
        r#"{"api_base":"","api_key":"os.environ/OPENAI_API_KEY","organization":"org-1","vertex_project":"proj","vertex_location":""}"#,
    )
    .unwrap();
    let environment = BTreeMap::from([
        ("OPENAI_API_KEY".to_owned(), "sk-env".to_owned()),
        ("OPENAI_BASE_URL".to_owned(), "http://env/v1".to_owned()),
        ("OPENAI_ORGANIZATION".to_owned(), "env-org".to_owned()),
    ]);
    let settings = section_settings::<batch_provider_create::Raw>(&module, &environment).unwrap();
    assert_eq!(settings.api_base, None);
    assert_eq!(settings.openai_base_url.as_deref(), Some("http://env/v1"));
    assert_eq!(settings.openai_api_base, None);
    assert_eq!(settings.api_key.as_ref().unwrap().expose_secret(), "sk-env");
    assert_eq!(
        settings.openai_api_key.as_ref().unwrap().expose_secret(),
        "sk-env"
    );
    assert_eq!(settings.organization.as_deref(), Some("org-1"));
    assert_eq!(settings.openai_organization.as_deref(), Some("env-org"));
    assert_eq!(settings.vertex_project.as_deref(), Some("proj"));
    assert_eq!(settings.vertex_location, None);
    assert!(settings.azure_key.is_none());
}

#[test]
fn request_read_settings_read_the_anthropic_environment() {
    let module = ModuleSettings::default();
    let environment = BTreeMap::from([
        ("ANTHROPIC_API_BASE".to_owned(), "http://a".to_owned()),
        ("ANTHROPIC_BASE_URL".to_owned(), "http://b".to_owned()),
        ("ANTHROPIC_API_KEY".to_owned(), "key".to_owned()),
    ]);
    let settings = section_settings::<batch_provider_read::Raw>(&module, &environment).unwrap();
    assert_eq!(settings.anthropic_base(), Some("http://a"));
    assert_eq!(
        settings.anthropic_api_key.as_ref().unwrap().expose_secret(),
        "key"
    );
    let fallback = BTreeMap::from([
        ("ANTHROPIC_API_BASE".to_owned(), String::new()),
        ("ANTHROPIC_BASE_URL".to_owned(), "http://b".to_owned()),
    ]);
    let empty = section_settings::<batch_provider_read::Raw>(&module, &fallback).unwrap();
    assert_eq!(empty.anthropic_base(), Some("http://b"));
    let none = section_settings::<batch_provider_read::Raw>(&module, &BTreeMap::new()).unwrap();
    assert_eq!(none.anthropic_base(), None);
}

#[test]
fn request_xai_settings_read_the_xai_environment() {
    let module: ModuleSettings = parse(r#"{"api_key":"generic"}"#).unwrap();
    let environment = BTreeMap::from([
        ("XAI_API_KEY".to_owned(), "xai-env".to_owned()),
        ("XAI_API_BASE".to_owned(), "http://xai".to_owned()),
    ]);
    let settings = section_settings::<xai_identity::Raw>(&module, &environment).unwrap();
    assert_eq!(
        settings.generic_api_key.as_ref().unwrap().expose_secret(),
        "generic"
    );
    assert_eq!(
        settings.environment_key.as_ref().unwrap().expose_secret(),
        "xai-env"
    );
    assert_eq!(settings.api_base.as_deref(), Some("http://xai"));
    assert!(settings.xai_key.is_none());
}

#[test]
fn request_create_hidden_params_take_the_callers_api_base() {
    let input: BatchProviderCreateInput = create_payload(CREATE_PAYLOAD).unwrap();
    let hidden = HiddenParams::new(
        input.custom_llm_provider.as_str(),
        input.model.as_deref(),
        input.optional_params.api_base.as_deref(),
    );
    assert_eq!(hidden.llm_provider, "openai");
    assert_eq!(hidden.model_id, None);
    assert_eq!(hidden.litellm_model_name.as_deref(), Some("gpt-4o"));
    assert_eq!(hidden.api_base.as_deref(), Some("http://local/v1"));
}

#[test]
fn request_create_hidden_params_default_the_provider_base() {
    let openai: BatchProviderCreateInput = create_payload(
        r#"{"completion_window":"24h","endpoint":"/v1/chat/completions","input_file_id":"f","custom_llm_provider":"openai","model":"gpt-4o"}"#,
    )
    .unwrap();
    let hidden = HiddenParams::new(
        openai.custom_llm_provider.as_str(),
        openai.model.as_deref(),
        openai.optional_params.api_base.as_deref(),
    );
    assert_eq!(hidden.api_base.as_deref(), Some("https://api.openai.com"));
    // with no model and the default provider, get_api_base resolves nothing and reports null
    let unmodeled: BatchProviderCreateInput = create_payload(
        r#"{"completion_window":"24h","endpoint":"/v1/chat/completions","input_file_id":"f","custom_llm_provider":"openai"}"#,
    )
    .unwrap();
    let unmodeled_hidden = HiddenParams::new(
        unmodeled.custom_llm_provider.as_str(),
        unmodeled.model.as_deref(),
        unmodeled.optional_params.api_base.as_deref(),
    );
    assert_eq!(unmodeled_hidden.api_base, None);
    let xai: BatchProviderCreateInput = create_payload(
        r#"{"completion_window":"24h","endpoint":"/v1/chat/completions","input_file_id":"f","custom_llm_provider":"xai","model":"xai/grok-2"}"#,
    )
    .unwrap();
    let xai_hidden = HiddenParams::new(
        xai.custom_llm_provider.as_str(),
        xai.model.as_deref(),
        xai.optional_params.api_base.as_deref(),
    );
    assert_eq!(xai_hidden.api_base.as_deref(), Some("https://api.x.ai/v1"));
    let proxy: BatchProviderCreateInput = create_payload(
        r#"{"completion_window":"24h","endpoint":"/v1/chat/completions","input_file_id":"f","custom_llm_provider":"litellm_proxy"}"#,
    )
    .unwrap();
    let proxy_hidden = HiddenParams::new(
        proxy.custom_llm_provider.as_str(),
        proxy.model.as_deref(),
        proxy.optional_params.api_base.as_deref(),
    );
    assert_eq!(proxy_hidden.api_base, None);
}

#[test]
fn request_read_hidden_params_take_the_callers_api_base() {
    let input: BatchProviderReadInput = read_payload(
        r#"{"batch_id":"b","provider":"openai","model":"gpt-4o","optional_params":{"api_base":"http://local/v1"}}"#,
    )
    .unwrap();
    let hidden = HiddenParams::new(
        input.provider.name(),
        input.model.as_deref(),
        input.optional_params.api_base.as_deref(),
    );
    assert_eq!(hidden.llm_provider, "openai");
    assert_eq!(hidden.model_id, None);
    assert_eq!(hidden.litellm_model_name.as_deref(), Some("gpt-4o"));
    assert_eq!(hidden.api_base.as_deref(), Some("http://local/v1"));
}

#[test]
fn request_read_hidden_params_default_the_provider_base() {
    let openai = read_call("openai", None);
    let openai_hidden = HiddenParams::new(
        openai.provider.name(),
        openai.model.as_deref(),
        openai.optional_params.api_base.as_deref(),
    );
    assert_eq!(openai_hidden.api_base, None);
    // xai reaches the payload only when the caller passed it, and get_api_base reports its base with or without a model
    let unmodeled_xai = read_call("xai", None);
    let unmodeled_xai_hidden = HiddenParams::new(
        unmodeled_xai.provider.name(),
        unmodeled_xai.model.as_deref(),
        unmodeled_xai.optional_params.api_base.as_deref(),
    );
    assert_eq!(
        unmodeled_xai_hidden.api_base.as_deref(),
        Some("https://api.x.ai/v1")
    );
    let xai = read_call("xai", Some("xai/grok-2"));
    let hidden = HiddenParams::new(
        xai.provider.name(),
        xai.model.as_deref(),
        xai.optional_params.api_base.as_deref(),
    );
    assert_eq!(hidden.api_base.as_deref(), Some("https://api.x.ai/v1"));
    let unknown = read_call("cohere", None);
    let unknown_hidden = HiddenParams::new(
        unknown.provider.name(),
        unknown.model.as_deref(),
        unknown.optional_params.api_base.as_deref(),
    );
    assert_eq!(unknown_hidden.api_base, None);
}

#[test]
fn request_answer_flattens_the_batch_and_hidden_params() {
    let batch: BatchObject = parse(BATCH_JSON).unwrap();
    let input: BatchProviderCreateInput = create_payload(CREATE_PAYLOAD).unwrap();
    let text = serde_json::to_string(&BatchAnswer {
        batch: &batch,
        hidden_params: HiddenParams::new(
            input.custom_llm_provider.as_str(),
            input.model.as_deref(),
            input.optional_params.api_base.as_deref(),
        ),
    })
    .unwrap();
    let view: AnswerView = parse(&text).unwrap();
    assert_eq!(view.id, "batch-1");
    assert_eq!(view.hidden_params.llm_provider, "openai");
    assert_eq!(
        view.hidden_params.litellm_model_name.as_deref(),
        Some("gpt-4o")
    );
    assert_eq!(
        view.hidden_params.api_base.as_deref(),
        Some("http://local/v1")
    );
    assert!(text.contains(r#""status":"validating""#));
    assert!(text.contains(r#""object":"batch""#));
}

#[test]
fn error_class_for_status_names_the_exception() {
    assert_eq!(class_for_status(400), "BadRequestError");
    assert_eq!(class_for_status(401), "AuthenticationError");
    assert_eq!(class_for_status(403), "PermissionDeniedError");
    assert_eq!(class_for_status(404), "NotFoundError");
    assert_eq!(class_for_status(422), "UnprocessableEntityError");
    assert_eq!(class_for_status(429), "RateLimitError");
    assert_eq!(class_for_status(500), "InternalServerError");
    assert_eq!(class_for_status(599), "InternalServerError");
    assert_eq!(class_for_status(499), "APIError");
    assert_eq!(class_for_status(600), "APIError");
}

#[test]
fn error_create_payload_names_the_unsupported_provider() {
    let error = BatchProviderCreateError::UnsupportedProvider {
        provider: BatchProvider::Bedrock,
    };
    let payload = create_error_payload(&error, "openai", Some("gpt-4o"));
    assert_eq!(
        payload,
        ErrorPayload {
            error_type: "BadRequestError",
            message: error.to_string(),
            status_code: Some(400),
            llm_provider: "bedrock".to_owned(),
            model: Some("n/a".to_owned()),
        }
    );
}

#[test]
fn error_create_payload_maps_a_provider_status() {
    let error = BatchProviderCreateError::Provider { status: 404 };
    let payload = create_error_payload(&error, "openai", Some("gpt-4o"));
    assert_eq!(
        payload,
        ErrorPayload {
            error_type: "NotFoundError",
            message: "the provider batch call failed with status 404".to_owned(),
            status_code: Some(404),
            llm_provider: "openai".to_owned(),
            model: Some("gpt-4o".to_owned()),
        }
    );
}

#[test]
fn error_create_payload_maps_a_limit() {
    let api_limits = ApiLimits {
        max_json_bytes: 1,
        max_event_bytes: 1,
        max_items: 0,
    };
    let limit = api_limits
        .check(ApiBoundariesInput::Json(JsonBoundary { known_bytes: 2 }))
        .unwrap_err();
    let expected = limit.fields();
    let payload = create_error_payload(&BatchProviderCreateError::Limit(limit), "openai", None);
    assert_eq!(payload.error_type, "APIError");
    assert_eq!(payload.status_code, Some(expected.status));
    assert_eq!(payload.message, limit.to_string());
    assert_eq!(payload.llm_provider, "openai");
    assert_eq!(payload.model, None);
}

#[test]
fn error_read_payload_maps_a_provider_status() {
    let failure = DispatchFailure::Status {
        status: 404,
        message: "Error code: 404 - no such batch".to_owned(),
    };
    let payload = read_error_payload(
        &BatchProviderReadError::Dispatch(failure),
        "openai",
        Some("gpt-4o"),
    );
    assert_eq!(
        payload,
        ErrorPayload {
            error_type: "NotFoundError",
            message: "Error code: 404 - no such batch".to_owned(),
            status_code: Some(404),
            llm_provider: "openai".to_owned(),
            model: Some("gpt-4o".to_owned()),
        }
    );
}

#[test]
fn error_read_payload_maps_a_connection_failure() {
    let failure = DispatchFailure::Connection("the provider is unreachable".to_owned());
    let payload = read_error_payload(&BatchProviderReadError::Dispatch(failure), "openai", None);
    assert_eq!(payload.error_type, "APIConnectionError");
    assert_eq!(payload.status_code, None);
    assert_eq!(payload.message, "the provider is unreachable");
    assert_eq!(payload.model, None);
}

#[test]
fn error_read_payload_maps_local_failures() {
    let timeout = BatchProviderReadError::Timeout(TimeoutError::NonFinite);
    let timeout_payload = read_error_payload(&timeout, "openai", None);
    assert_eq!(timeout_payload.error_type, "BadRequestError");
    assert_eq!(timeout_payload.status_code, Some(400));
    assert_eq!(timeout_payload.message, "timeout must be finite");

    let credentials = read_error_payload(
        &BatchProviderReadError::<DispatchFailure>::InvalidVertexCredentials,
        "vertex_ai",
        None,
    );
    assert_eq!(credentials.error_type, "BadRequestError");
    assert_eq!(credentials.status_code, Some(400));
    assert_eq!(credentials.llm_provider, "vertex_ai");

    let unsupported = read_error_payload(
        &BatchProviderReadError::<DispatchFailure>::Unsupported {
            provider: "mistral".to_owned(),
        },
        "mistral",
        None,
    );
    assert_eq!(unsupported.error_type, "BadRequestError");
    assert_eq!(unsupported.status_code, Some(400));
    assert!(unsupported.message.contains("mistral"));
}

#[test]
fn error_read_payload_maps_an_unwired_provider() {
    let failure = DispatchFailure::Unsupported {
        provider: "azure".to_owned(),
    };
    let payload = read_error_payload(&BatchProviderReadError::Dispatch(failure), "azure", None);
    assert_eq!(payload.error_type, "BadRequestError");
    assert_eq!(payload.status_code, Some(400));
    assert!(payload.message.contains("azure"));
}

#[test]
fn error_read_payload_maps_a_request_failure() {
    let failure = DispatchFailure::Request(MissingBatchField::Missing { field: "batch_id" });
    let payload = read_error_payload(&BatchProviderReadError::Dispatch(failure), "openai", None);
    assert_eq!(payload.error_type, "BadRequestError");
    assert_eq!(payload.status_code, Some(400));
    assert!(payload.message.contains("batch_id"));
}

#[test]
fn error_read_payload_maps_a_timeout_failure() {
    let failure = DispatchFailure::Timeout(SdkTimeoutError::Invalid { seconds: -1.0_f64 });
    let payload = read_error_payload(&BatchProviderReadError::Dispatch(failure), "openai", None);
    assert_eq!(payload.error_type, "BadRequestError");
    assert_eq!(payload.status_code, Some(400));
    assert!(payload.message.contains("-1"));
}

#[test]
fn error_read_payload_maps_a_limit() {
    let api_limits = ApiLimits {
        max_json_bytes: 0,
        max_event_bytes: 1,
        max_items: 1,
    };
    let limit = api_limits
        .check(ApiBoundariesInput::Json(JsonBoundary { known_bytes: 3 }))
        .unwrap_err();
    let payload = read_error_payload(&BatchProviderReadError::Limits(limit), "openai", None);
    assert_eq!(payload.error_type, "APIError");
    assert_eq!(payload.status_code, Some(413));
}

#[test]
fn error_dispatch_fields_maps_each_failure() {
    assert_eq!(
        dispatch_fields(&DispatchFailure::Status {
            status: 429,
            message: "slow down".to_owned(),
        }),
        ("RateLimitError", Some(429), "slow down".to_owned())
    );
    assert_eq!(
        dispatch_fields(&DispatchFailure::Connection("down".to_owned())),
        ("APIConnectionError", None, "down".to_owned())
    );
    assert_eq!(
        dispatch_fields(&DispatchFailure::Unsupported {
            provider: "azure".to_owned(),
        }),
        (
            "BadRequestError",
            Some(400),
            "retrieving a batch from provider `azure` is not available in this build".to_owned(),
        )
    );
}

#[test]
fn error_payload_is_the_shape_python_reads() {
    let payload = ErrorPayload {
        error_type: "NotFoundError",
        message: "no such batch".to_owned(),
        status_code: Some(404),
        llm_provider: "openai".to_owned(),
        model: Some("gpt-4o".to_owned()),
    };
    assert_eq!(
        serde_json::to_string(&payload).unwrap(),
        r#"{"type":"NotFoundError","message":"no such batch","status_code":404,"llm_provider":"openai","model":"gpt-4o"}"#
    );
}
