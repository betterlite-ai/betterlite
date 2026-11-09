use std::collections::BTreeMap;

use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::langsmith_payloads::credentials::{
    CredentialError, CredentialParams, Credentials, DEFAULT_BASE_URL, DEFAULT_PROJECT,
    credentials_for_request, get_credentials_from_env,
};
use betterlite_logging::langsmith_payloads::ids::{IdSource, RunIds, ensure_required_ids};
use betterlite_logging::langsmith_payloads::metadata::{DEFAULT_RUN_NAME, extract_metadata_fields};
use betterlite_logging::langsmith_payloads::outputs::build_outputs_with_usage;
use betterlite_logging::langsmith_payloads::prepare::{
    PayloadError, Prepare, RunData, prepare_log_data,
};
use betterlite_logging::langsmith_payloads::redaction::{build_extra_metadata, redact_metadata};
use betterlite_logging::langsmith_payloads::sampling::{SamplingError, sampling_rate_for_request};
use betterlite_models::decisions::Json;
use chrono::{DateTime, Duration, TimeZone as _, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};

const PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 30,
    "prompt_tokens": 10,
    "completion_tokens": 20,
    "startTime": 1704067200.0,
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.0,
    "model_map_information": {"model_map_key": "", "model_map_value": null},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": ["team-a"],
    "model_parameters": {},
    "hidden_params": {},
    "response": {"id": "chatcmpl-1"},
    "error_str": null
}"#;

struct FixedIds(&'static str);

impl IdSource for FixedIds {
    fn next_id(&self) -> String {
        self.0.to_owned()
    }
}

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("a JSON value")
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn metadata(text_value: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text_value).expect("a metadata object")
}

fn payload_with(changes: &[(&str, Json)]) -> StandardLoggingPayload {
    let mut raw = json(PAYLOAD);
    let Json::Object(fields) = &mut raw else {
        panic!("the payload is an object");
    };
    for (name, value) in changes {
        fields.insert((*name).to_owned(), value.clone());
    }
    let encoded = serde_json::to_string(&raw).expect("the payload encodes");
    serde_json::from_str(&encoded).expect("a standard logging payload")
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5)
        .unwrap()
        .checked_add_signed(Duration::microseconds(123_456))
        .expect("a representable instant")
}

fn test_credentials(project: &str) -> Credentials {
    Credentials {
        api_key: None,
        project: project.to_owned(),
        base_url: DEFAULT_BASE_URL.to_owned(),
        tenant_id: None,
    }
}

fn prepare<'event>(
    payload: &'event StandardLoggingPayload,
    metadata: &'event IndexMap<String, Json>,
    credentials: &'event Credentials,
    redact: bool,
) -> Prepare<'event> {
    Prepare {
        standard_logging_object: Some(payload),
        litellm_metadata: metadata,
        credentials,
        default_run_name: DEFAULT_RUN_NAME,
        redact_user_api_key_info: redact,
    }
}

fn success_run() -> RunData {
    let payload = payload_with(&[(
        "metadata",
        json(
            r#"{"user_api_key_hash": "hash", "requester_metadata": {"user_api_key": "secret", "session_id": "nested-session"}}"#,
        ),
    )]);
    let litellm = metadata(
        r#"{"run_name": "my-run", "project_name": "my-project", "session_id": "session-1"}"#,
    );
    let credentials = test_credentials(DEFAULT_PROJECT);
    let request = prepare(&payload, &litellm, &credentials, true);
    prepare_log_data(&request, now(), &FixedIds("generated-id")).expect("run")
}

fn assert_run_identity(run: &RunData) {
    assert!(run.parent_run_id.is_none());
    assert_eq!(run.id, "generated-id");
    assert_eq!(run.trace_id, "generated-id");
    assert_eq!(run.dotted_order, "20240102T030405123456Zgenerated-id");
}

fn assert_run_outputs(run: &RunData) {
    assert_eq!(run.start_time, Decimal::from(1_704_067_200_i64));
    assert_eq!(run.end_time, Decimal::new(17_040_672_015, 1));
    assert_eq!(run.tags, vec![text("team-a")]);
    assert_eq!(run.outputs.get("id"), Some(&text("chatcmpl-1")));
}

fn assert_run_metadata(run: &RunData) {
    assert_eq!(run.extra.get("session_id"), Some(&text("nested-session")));
    assert!(run.extra.get("user_api_key_hash").is_none());
    let Some(Json::Object(input_metadata)) = run.inputs.get("metadata") else {
        panic!("the inputs carry the metadata");
    };
    assert!(input_metadata.get("user_api_key_hash").is_none());
    assert_eq!(
        input_metadata.get("requester_metadata"),
        Some(&json(r#"{"session_id": "nested-session"}"#))
    );
    let value = serde_json::to_value(run).expect("run json");
    assert!(value.get("parent_run_id").is_none());
}

#[test]
fn api_key_env() {
    let environment = env(&[("LANGSMITH_API_KEY", "env-key")]);
    let credentials = get_credentials_from_env(&environment, &CredentialParams::default(), true);
    assert_eq!(
        credentials
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("env-key")
    );
}

#[test]
fn api_key_param() {
    let environment = env(&[("LANGSMITH_API_KEY", "env-key")]);
    let params = CredentialParams {
        api_key: Some(SecretString::from("param-key")),
        project: None,
        base_url: None,
        tenant_id: None,
    };
    let credentials = get_credentials_from_env(&environment, &params, true);
    assert_eq!(
        credentials
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("param-key")
    );
}

#[test]
fn project_default() {
    let defaults = get_credentials_from_env(&env(&[]), &CredentialParams::default(), true);
    assert_eq!(defaults.project, DEFAULT_PROJECT);
    assert_eq!(defaults.project, "litellm-completion");
    let blank = env(&[("LANGSMITH_PROJECT", "")]);
    let blank_credentials = get_credentials_from_env(&blank, &CredentialParams::default(), true);
    assert_eq!(blank_credentials.project, DEFAULT_PROJECT);
}

#[test]
fn project_env() {
    let environment = env(&[("LANGSMITH_PROJECT", "team-project")]);
    let credentials = get_credentials_from_env(&environment, &CredentialParams::default(), true);
    assert_eq!(credentials.project, "team-project");
}

#[test]
fn base_url_default() {
    let credentials = get_credentials_from_env(&env(&[]), &CredentialParams::default(), true);
    assert_eq!(credentials.base_url, DEFAULT_BASE_URL);
    assert_eq!(credentials.base_url, "https://api.smith.langchain.com");
}

#[test]
fn base_url_env() {
    let environment = env(&[("LANGSMITH_BASE_URL", "https://langsmith.example")]);
    let credentials = get_credentials_from_env(&environment, &CredentialParams::default(), true);
    assert_eq!(credentials.base_url, "https://langsmith.example");
}

#[test]
fn tenant_id_none() {
    let credentials = get_credentials_from_env(&env(&[]), &CredentialParams::default(), true);
    assert!(credentials.tenant_id.is_none());
}

#[test]
fn tenant_id_env() {
    let environment = env(&[("LANGSMITH_TENANT_ID", "tenant-1")]);
    let credentials = get_credentials_from_env(&environment, &CredentialParams::default(), true);
    assert_eq!(credentials.tenant_id.as_deref(), Some("tenant-1"));
}

#[test]
fn explicit_base_url_blocks_env() {
    let environment = env(&[
        ("LANGSMITH_API_KEY", "env-key"),
        ("LANGSMITH_PROJECT", "env-project"),
        ("LANGSMITH_BASE_URL", "https://env.example"),
        ("LANGSMITH_TENANT_ID", "env-tenant"),
    ]);
    let params = CredentialParams {
        api_key: None,
        project: None,
        base_url: Some("https://explicit.example".to_owned()),
        tenant_id: None,
    };
    let credentials = get_credentials_from_env(&environment, &params, false);
    assert!(credentials.api_key.is_none());
    assert_eq!(credentials.project, DEFAULT_PROJECT);
    assert_eq!(credentials.base_url, "https://explicit.example");
    assert!(credentials.tenant_id.is_none());
}

#[test]
fn allow_env_true_uses_env() {
    let environment = env(&[
        ("LANGSMITH_API_KEY", "env-key"),
        ("LANGSMITH_PROJECT", "env-project"),
        ("LANGSMITH_BASE_URL", "https://env.example"),
        ("LANGSMITH_TENANT_ID", "env-tenant"),
    ]);
    let params = CredentialParams {
        api_key: None,
        project: None,
        base_url: Some("https://explicit.example".to_owned()),
        tenant_id: None,
    };
    let credentials = get_credentials_from_env(&environment, &params, true);
    assert_eq!(
        credentials
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("env-key")
    );
    assert_eq!(credentials.project, "env-project");
    assert_eq!(credentials.base_url, "https://explicit.example");
    assert_eq!(credentials.tenant_id.as_deref(), Some("env-tenant"));
}

#[test]
fn extract_run_name() {
    let credentials = test_credentials(DEFAULT_PROJECT);
    let named = metadata(r#"{"run_name": "my-run"}"#);
    let named_fields = extract_metadata_fields(&named, &credentials, DEFAULT_RUN_NAME);
    assert_eq!(named_fields.run_name, "my-run");
    let absent = metadata("{}");
    let absent_fields = extract_metadata_fields(&absent, &credentials, DEFAULT_RUN_NAME);
    assert_eq!(absent_fields.run_name, DEFAULT_RUN_NAME);
    assert_eq!(DEFAULT_RUN_NAME, "LLMRun");
}

#[test]
fn extract_run_id() {
    let credentials = test_credentials(DEFAULT_PROJECT);
    let both = metadata(r#"{"id": "id-1", "run_id": "id-2"}"#);
    let id_fields = extract_metadata_fields(&both, &credentials, DEFAULT_RUN_NAME);
    assert_eq!(id_fields.run_id.as_deref(), Some("id-1"));
    let fallback = metadata(r#"{"run_id": "id-2"}"#);
    let fallback_fields = extract_metadata_fields(&fallback, &credentials, DEFAULT_RUN_NAME);
    assert_eq!(fallback_fields.run_id.as_deref(), Some("id-2"));
    let absent = metadata("{}");
    let absent_fields = extract_metadata_fields(&absent, &credentials, DEFAULT_RUN_NAME);
    assert!(absent_fields.run_id.is_none());
}

#[test]
fn extract_parent_run_id() {
    let credentials = test_credentials(DEFAULT_PROJECT);
    let present = metadata(r#"{"parent_run_id": "parent-1"}"#);
    let present_fields = extract_metadata_fields(&present, &credentials, DEFAULT_RUN_NAME);
    assert_eq!(present_fields.parent_run_id.as_deref(), Some("parent-1"));
    let absent = metadata("{}");
    let absent_fields = extract_metadata_fields(&absent, &credentials, DEFAULT_RUN_NAME);
    assert!(absent_fields.parent_run_id.is_none());
}

#[test]
fn extract_trace_id() {
    let credentials = test_credentials(DEFAULT_PROJECT);
    let present = metadata(r#"{"trace_id": "trace-1"}"#);
    let present_fields = extract_metadata_fields(&present, &credentials, DEFAULT_RUN_NAME);
    assert_eq!(present_fields.trace_id.as_deref(), Some("trace-1"));
    let absent = metadata("{}");
    let absent_fields = extract_metadata_fields(&absent, &credentials, DEFAULT_RUN_NAME);
    assert!(absent_fields.trace_id.is_none());
}

#[test]
fn extract_session_id() {
    let credentials = test_credentials(DEFAULT_PROJECT);
    let present = metadata(r#"{"session_id": "session-1"}"#);
    let present_fields = extract_metadata_fields(&present, &credentials, DEFAULT_RUN_NAME);
    assert_eq!(present_fields.session_id.as_deref(), Some("session-1"));
    let absent = metadata("{}");
    let absent_fields = extract_metadata_fields(&absent, &credentials, DEFAULT_RUN_NAME);
    assert!(absent_fields.session_id.is_none());
}

#[test]
fn extract_dotted_order() {
    let credentials = test_credentials(DEFAULT_PROJECT);
    let present = metadata(r#"{"dotted_order": "20240102T030405123456Zrun-1"}"#);
    let present_fields = extract_metadata_fields(&present, &credentials, DEFAULT_RUN_NAME);
    assert_eq!(
        present_fields.dotted_order.as_deref(),
        Some("20240102T030405123456Zrun-1")
    );
    let absent = metadata("{}");
    let absent_fields = extract_metadata_fields(&absent, &credentials, DEFAULT_RUN_NAME);
    assert!(absent_fields.dotted_order.is_none());
}

#[test]
fn redact_top_level() {
    let original = metadata(
        r#"{"user_api_key_hash": "hash", "user_api_key_alias": "alias", "model": "gpt-4o"}"#,
    );
    let redacted = redact_metadata(&original, true);
    assert!(redacted.get("user_api_key_hash").is_none());
    assert!(redacted.get("user_api_key_alias").is_none());
    assert_eq!(redacted.get("model"), Some(&text("gpt-4o")));
    let kept = redact_metadata(&original, false);
    assert_eq!(kept, original);
}

#[test]
fn redact_nested_requester() {
    let original = metadata(
        r#"{"model": "gpt-4o", "requester_metadata": {"user_api_key": "secret", "session_id": "session-1"}}"#,
    );
    let redacted = redact_metadata(&original, true);
    let Some(Json::Object(requester)) = redacted.get("requester_metadata") else {
        panic!("the requester metadata is an object");
    };
    assert!(requester.get("user_api_key").is_none());
    assert_eq!(requester.get("session_id"), Some(&text("session-1")));
    let plain = metadata(r#"{"requester_metadata": "not-a-dict"}"#);
    let plain_redacted = redact_metadata(&plain, true);
    assert_eq!(
        plain_redacted.get("requester_metadata"),
        Some(&text("not-a-dict"))
    );
}

#[test]
fn elevate_session_id() {
    let original = metadata(r#"{"requester_metadata": {"session_id": "session-1"}}"#);
    let extra = build_extra_metadata(original, false);
    assert_eq!(extra.get("session_id"), Some(&text("session-1")));
    let top_level =
        metadata(r#"{"session_id": "top-level", "requester_metadata": {"session_id": "nested"}}"#);
    let top_extra = build_extra_metadata(top_level, false);
    assert_eq!(top_extra.get("session_id"), Some(&text("top-level")));
}

#[test]
fn elevate_thread_id() {
    let original = metadata(r#"{"requester_metadata": {"thread_id": "thread-1"}}"#);
    let extra = build_extra_metadata(original, false);
    assert_eq!(extra.get("thread_id"), Some(&text("thread-1")));
}

#[test]
fn elevate_conversation_id() {
    let original = metadata(r#"{"requester_metadata": {"conversation_id": "conversation-1"}}"#);
    let extra = build_extra_metadata(original, false);
    assert_eq!(extra.get("conversation_id"), Some(&text("conversation-1")));
}

#[test]
fn extra_metadata_redacted() {
    let original = metadata(
        r#"{"user_api_key_hash": "hash", "requester_metadata": {"user_api_key": "secret", "session_id": "session-1"}}"#,
    );
    let extra = build_extra_metadata(original, true);
    assert!(extra.get("user_api_key_hash").is_none());
    assert_eq!(extra.get("session_id"), Some(&text("session-1")));
    let Some(Json::Object(requester)) = extra.get("requester_metadata") else {
        panic!("the requester metadata is an object");
    };
    assert!(requester.get("user_api_key").is_none());
    assert_eq!(requester.get("session_id"), Some(&text("session-1")));
}

#[test]
fn outputs_dict_response() {
    let payload = payload_with(&[("response", json(r#"{"id": "chatcmpl-1", "choices": []}"#))]);
    let outputs = build_outputs_with_usage(&payload).expect("outputs");
    assert_eq!(outputs.get("id"), Some(&text("chatcmpl-1")));
    assert!(outputs.get("output").is_none());
    assert!(outputs.get("usage_metadata").is_some());
}

#[test]
fn outputs_non_dict_response() {
    let text_payload = payload_with(&[("response", text("the answer"))]);
    let text_outputs = build_outputs_with_usage(&text_payload).expect("outputs");
    assert_eq!(text_outputs.get("output"), Some(&text("the answer")));
    let null_payload = payload_with(&[("response", Json::Null)]);
    let null_outputs = build_outputs_with_usage(&null_payload).expect("outputs");
    assert_eq!(null_outputs.get("output"), Some(&Json::Null));
}

#[test]
fn usage_metadata_tokens() {
    let payload = payload_with(&[]);
    let outputs = build_outputs_with_usage(&payload).expect("outputs");
    let Some(Json::Object(usage)) = outputs.get("usage_metadata") else {
        panic!("the outputs carry usage metadata");
    };
    assert_eq!(usage.get("input_tokens"), Some(&json("10")));
    assert_eq!(usage.get("output_tokens"), Some(&json("20")));
    assert_eq!(usage.get("total_tokens"), Some(&json("30")));
}

#[test]
fn usage_metadata_cost() {
    let payload = payload_with(&[("response_cost", json("2.5"))]);
    let outputs = build_outputs_with_usage(&payload).expect("outputs");
    let Some(Json::Object(usage)) = outputs.get("usage_metadata") else {
        panic!("the outputs carry usage metadata");
    };
    assert_eq!(usage.get("total_cost"), Some(&json("2.5")));
}

#[test]
fn ensure_run_id_generated() {
    let ids = RunIds::default();
    let required = ensure_required_ids(&ids, None, now(), &FixedIds("generated-id"));
    assert_eq!(required.id, "generated-id");
    assert_eq!(required.trace_id, "generated-id");
    assert_eq!(required.dotted_order, "20240102T030405123456Zgenerated-id");
}

#[test]
fn ensure_trace_id_root() {
    let ids = RunIds {
        id: None,
        parent_run_id: None,
        trace_id: Some("stale-trace".to_owned()),
        dotted_order: None,
    };
    let required = ensure_required_ids(&ids, Some("run-1"), now(), &FixedIds("unused"));
    assert_eq!(required.id, "run-1");
    assert_eq!(required.trace_id, "run-1");
}

#[test]
fn ensure_dotted_order_root() {
    let ids = RunIds::default();
    let required = ensure_required_ids(&ids, Some("run-1"), now(), &FixedIds("unused"));
    assert_eq!(required.dotted_order, "20240102T030405123456Zrun-1");
}

#[test]
fn trace_id_non_root() {
    let ids = RunIds {
        id: Some("run-1".to_owned()),
        parent_run_id: Some("parent-1".to_owned()),
        trace_id: Some("trace-1".to_owned()),
        dotted_order: Some("20240102T030405123456Zrun-1".to_owned()),
    };
    let required = ensure_required_ids(&ids, Some("run-1"), now(), &FixedIds("unused"));
    assert_eq!(required.trace_id, "trace-1");
    assert_eq!(required.dotted_order, "20240102T030405123456Zrun-1");
}

#[test]
fn sampling_rate_dynamic() {
    let dynamic = json(r#"{"langsmith_sampling_rate": 0.25}"#);
    let rate = sampling_rate_for_request(1.0, Some(&dynamic)).expect("rate");
    assert_eq!(rate.to_bits(), 0.25_f64.to_bits());
    let decimal_text = json(r#"{"langsmith_sampling_rate": "0.25"}"#);
    let text_rate = sampling_rate_for_request(1.0, Some(&decimal_text)).expect("text rate");
    assert_eq!(text_rate.to_bits(), 0.25_f64.to_bits());
    let wrong_type = json(r#"{"langsmith_sampling_rate": "half"}"#);
    let error = sampling_rate_for_request(1.0, Some(&wrong_type)).expect_err("a non-numeric rate");
    assert!(matches!(error, SamplingError::RateType));
}

#[test]
fn sampling_rate_default() {
    let empty = json("{}");
    let empty_rate = sampling_rate_for_request(0.5, Some(&empty)).expect("rate");
    assert_eq!(empty_rate.to_bits(), 0.5_f64.to_bits());
    let absent_rate = sampling_rate_for_request(0.5, None).expect("rate");
    assert_eq!(absent_rate.to_bits(), 0.5_f64.to_bits());
    let null = json(r#"{"langsmith_sampling_rate": null}"#);
    let null_rate = sampling_rate_for_request(0.5, Some(&null)).expect("rate");
    assert_eq!(null_rate.to_bits(), 0.5_f64.to_bits());
}

#[test]
fn select_dynamic_credentials() {
    let environment = env(&[]);
    let defaults = test_credentials(DEFAULT_PROJECT);
    let dynamic = json(
        r#"{"langsmith_api_key": "dynamic-key", "langsmith_project": "dynamic-project", "langsmith_base_url": "https://dynamic.example", "langsmith_tenant_id": "dynamic-tenant"}"#,
    );
    let selected = credentials_for_request(&environment, &defaults, Some(&dynamic))
        .expect("dynamic credentials");
    assert_eq!(
        selected.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("dynamic-key")
    );
    assert_eq!(selected.project, "dynamic-project");
    assert_eq!(selected.base_url, "https://dynamic.example");
    assert_eq!(selected.tenant_id.as_deref(), Some("dynamic-tenant"));
    let wrong_type = json(r#"{"langsmith_api_key": 7}"#);
    let error = credentials_for_request(&environment, &defaults, Some(&wrong_type))
        .expect_err("a non-string credential");
    assert!(matches!(
        error,
        CredentialError::FieldType {
            field: "langsmith_api_key"
        }
    ));
}

#[test]
fn select_default_credentials() {
    let environment = env(&[("LANGSMITH_API_KEY", "env-key")]);
    let defaults = test_credentials("default-project");
    let selected = credentials_for_request(&environment, &defaults, None).expect("credentials");
    assert!(selected.api_key.is_none());
    assert_eq!(selected.project, "default-project");
    assert_eq!(selected.base_url, DEFAULT_BASE_URL);
}

#[test]
fn select_credentials_isolation() {
    let environment = env(&[("LANGSMITH_API_KEY", "env-key")]);
    let defaults = test_credentials(DEFAULT_PROJECT);
    let isolated = json(r#"{"langsmith_base_url": "https://explicit.example"}"#);
    let isolated_selected = credentials_for_request(&environment, &defaults, Some(&isolated))
        .expect("isolated credentials");
    assert!(isolated_selected.api_key.is_none());
    assert_eq!(isolated_selected.project, DEFAULT_PROJECT);
    let open = json(r#"{"langsmith_project": "dynamic-project"}"#);
    let open_selected =
        credentials_for_request(&environment, &defaults, Some(&open)).expect("credentials");
    assert_eq!(
        open_selected
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("env-key")
    );
    assert_eq!(open_selected.project, "dynamic-project");
}

#[test]
fn dot_order_format() {
    let ids = RunIds::default();
    let microseconds = ensure_required_ids(&ids, Some("run-1"), now(), &FixedIds("unused"));
    assert_eq!(microseconds.dotted_order, "20240102T030405123456Zrun-1");
    let whole_second = Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap();
    let zero_padded = ensure_required_ids(&ids, Some("run-1"), whole_second, &FixedIds("unused"));
    assert_eq!(zero_padded.dotted_order, "20240102T030405000000Zrun-1");
}

#[test]
fn prepare_success_run() {
    let run = success_run();
    assert_eq!(run.name, "my-run");
    assert_eq!(run.run_type, "llm");
    assert_eq!(run.session_name, "my-project");
    assert_eq!(run.session_id.as_deref(), Some("session-1"));
    assert!(run.error.is_none());
    assert_run_identity(&run);
    assert_run_outputs(&run);
    assert_run_metadata(&run);
}

#[test]
fn prepare_failure_run() {
    let payload = payload_with(&[
        ("status", text("failure")),
        ("error_str", text("provider exploded")),
        ("response", Json::Null),
    ]);
    let litellm = metadata("{}");
    let credentials = test_credentials(DEFAULT_PROJECT);
    let request = prepare(&payload, &litellm, &credentials, false);
    let run = prepare_log_data(&request, now(), &FixedIds("generated-id")).expect("run");
    assert_eq!(run.error.as_deref(), Some("provider exploded"));
    assert_eq!(run.outputs.get("output"), Some(&Json::Null));
    assert_eq!(run.id, "generated-id");
    assert_eq!(run.trace_id, "generated-id");
    assert_eq!(run.name, DEFAULT_RUN_NAME);
    assert_eq!(run.session_name, DEFAULT_PROJECT);
}

#[test]
fn prepare_with_all_ids() {
    let payload = payload_with(&[]);
    let litellm = metadata(
        r#"{"id": "run-1", "parent_run_id": "parent-1", "trace_id": "trace-1", "session_id": "session-1", "dotted_order": "20240101T000000000000Zrun-1"}"#,
    );
    let credentials = test_credentials(DEFAULT_PROJECT);
    let first_request = prepare(&payload, &litellm, &credentials, false);
    let first_run = prepare_log_data(&first_request, now(), &FixedIds("unused")).expect("run");
    assert_eq!(first_run.id, "run-1");
    assert_eq!(first_run.parent_run_id.as_deref(), Some("parent-1"));
    assert_eq!(first_run.trace_id, "trace-1");
    assert_eq!(first_run.session_id.as_deref(), Some("session-1"));
    assert_eq!(first_run.dotted_order, "20240101T000000000000Zrun-1");
    let suppressed = metadata(
        r#"{"id": "run-1", "parent_run_id": "parent-1", "trace_id": "trace-1", "session_id": "trace-1", "dotted_order": "20240101T000000000000Zrun-1"}"#,
    );
    let suppressed_request = prepare(&payload, &suppressed, &credentials, false);
    let suppressed_run =
        prepare_log_data(&suppressed_request, now(), &FixedIds("unused")).expect("run");
    assert!(suppressed_run.session_id.is_none());
    assert_eq!(suppressed_run.trace_id, "trace-1");
}

#[test]
fn prepare_with_error_field() {
    let success_payload = payload_with(&[("error_str", text("late failure"))]);
    let litellm = metadata("{}");
    let credentials = test_credentials(DEFAULT_PROJECT);
    let success_request = prepare(&success_payload, &litellm, &credentials, false);
    let success_run =
        prepare_log_data(&success_request, now(), &FixedIds("generated-id")).expect("run");
    assert!(success_run.error.is_none());
    let failure_payload = payload_with(&[("status", text("failure"))]);
    let failure_request = prepare(&failure_payload, &litellm, &credentials, false);
    let failure_run =
        prepare_log_data(&failure_request, now(), &FixedIds("generated-id")).expect("run");
    assert!(failure_run.error.is_none());
}

#[test]
fn prepare_missing_payload_raises() {
    let credentials = test_credentials(DEFAULT_PROJECT);
    let metadata = IndexMap::new();
    let request = Prepare {
        standard_logging_object: None,
        litellm_metadata: &metadata,
        credentials: &credentials,
        default_run_name: DEFAULT_RUN_NAME,
        redact_user_api_key_info: false,
    };
    let error =
        prepare_log_data(&request, now(), &FixedIds("unused")).expect_err("missing payload");
    assert!(matches!(error, PayloadError::MissingStandardLoggingObject));
}
