#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::net::TcpListener as StdTcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::sections::grayswan::{Configured, OnFlaggedAction, ReasoningMode};
use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use reqwest::StatusCode;
use reqwest::header::HeaderValue;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::Builder;
use tracing::subscriber::with_default;

use super::GraySwanGuardrail;
use super::config_model::{GraySwanConfigOptionalParams, GraySwanGuardrailConfigModel};
use super::construction::{DEFAULT_API_BASE, GraySwanGuardrailSetupError};
use super::error::{GRAYSWAN_BLOCK_ERROR_MSG, GraySwanFailure, GraySwanGuardrailAPIError};
use super::format::build_messages;
use super::hooks::hook_name;
use super::wire::{
    GraySwanResponse, GraySwanRuleNumber, GraySwanVerdict, GraySwanViolatedRule,
    GraySwanViolatedRuleFields,
};
use crate::generic_guardrail_contracts::{
    GuardrailExtraValue, GuardrailInputType, GuardrailMessage,
};
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::third_party_init_a::grayswan::GraySwanArgs;

#[path = "grayswan_monitoring/tests.rs"]
mod grayswan_monitoring;

fn configured(api_base: &str, action: OnFlaggedAction, fail_open: bool) -> Configured {
    Configured {
        guardrail_name: "safety".to_owned(),
        api_key: SecretString::from("secret"),
        api_base: api_base.to_owned(),
        on_flagged_action: action,
        violation_threshold: 0.5,
        reasoning_mode: None,
        categories: BTreeMap::new(),
        policy_id: String::new(),
        fail_open,
        guardrail_timeout: Duration::from_secs(30),
        streaming_end_of_stream_only: false,
        streaming_sampling_rate: 5,
    }
}

fn base() -> GuardrailBase {
    GuardrailBase::new(GuardrailSettings::default(), true).unwrap()
}

fn guardrail(action: OnFlaggedAction) -> GraySwanGuardrail {
    let mut guardrail_base = base();
    guardrail_base.settings.guardrail_name = Some("safety".to_owned());
    GraySwanGuardrail {
        settings: configured("http://example", action, true),
        base: guardrail_base,
        client: reqwest::Client::default(),
    }
}

fn message(role: &str, content: &str) -> GuardrailMessage {
    let mut extra = BTreeMap::new();
    extra.insert(
        "content".to_owned(),
        GuardrailExtraValue::Text(content.to_owned()),
    );
    GuardrailMessage {
        role: role.to_owned(),
        extra,
    }
}

fn plain_message(role: &str, content: &str) -> GuardrailValue {
    let mut fields = BTreeMap::new();
    for (name, value) in [("role", role), ("content", content)] {
        fields.insert(name.to_owned(), GuardrailValue::Text(value.to_owned()));
    }
    GuardrailValue::Object(fields)
}

fn field<'row>(row: &'row GuardrailValue, key: &str) -> Option<&'row GuardrailValue> {
    let GuardrailValue::Object(fields) = row else {
        return None;
    };
    fields.get(key)
}

fn text_field(row: &GuardrailValue, key: &str) -> Option<String> {
    match field(row, key) {
        Some(GuardrailValue::Text(text)) => Some(text.clone()),
        _ => None,
    }
}

fn response(violation: f64) -> GraySwanResponse {
    GraySwanResponse {
        violation,
        violated_rule_descriptions: Vec::new(),
        mutation: None,
        ipi: None,
    }
}

#[test]
fn grayswan_build_messages_prepends_context_and_uses_request_role() {
    let context = vec![plain_message("assistant", "old")];
    let messages = build_messages(
        &["new".to_owned()],
        &[],
        GuardrailInputType::Request,
        context,
    );
    assert_eq!(messages.len(), 2);
    assert_eq!(
        text_field(&messages[0], "role").as_deref(),
        Some("assistant")
    );
    assert_eq!(text_field(&messages[1], "role").as_deref(), Some("user"));
}

#[test]
fn grayswan_build_messages_merges_single_text_tail_with_tool_calls() {
    let tool_calls = vec![GuardrailExtraValue::Object(BTreeMap::from([(
        "id".to_owned(),
        GuardrailExtraValue::Text("call_1".to_owned()),
    )]))];
    let messages = build_messages(
        &["final answer".to_owned()],
        &tool_calls,
        GuardrailInputType::Response,
        Vec::new(),
    );
    assert_eq!(messages.len(), 1);
    assert_eq!(
        text_field(&messages[0], "role").as_deref(),
        Some("assistant")
    );
    assert!(field(&messages[0], "tool_calls").is_some());
    assert_eq!(
        field(&messages[0], "content"),
        Some(&GuardrailValue::Text("final answer".to_owned()))
    );
}

#[test]
fn grayswan_build_messages_appends_a_separate_tool_calls_message_for_multiple_texts() {
    let tool_calls = vec![GuardrailExtraValue::Text("call".to_owned())];
    let messages = build_messages(
        &["first".to_owned(), "second".to_owned()],
        &tool_calls,
        GuardrailInputType::Response,
        Vec::new(),
    );
    assert_eq!(messages.len(), 3);
    assert_eq!(
        text_field(&messages[2], "role").as_deref(),
        Some("assistant")
    );
    assert!(field(&messages[2], "tool_calls").is_some());
    assert!(field(&messages[2], "content").is_none());
}

#[test]
fn grayswan_verdict_at_threshold_blocks() {
    let error = guardrail(OnFlaggedAction::Block)
        .process_response(&response(0.5), GuardrailInputType::Request, &[], &[])
        .expect_err("threshold violation");
    assert!(matches!(error.cause, Some(GraySwanFailure::Blocked { .. })));
}

#[test]
fn grayswan_block_error_preserves_the_detection_fields() {
    let mut scan_response = response(0.9);
    scan_response.violated_rule_descriptions = vec![structured_rule(
        Some(GraySwanRuleNumber::Integer(6)),
        Some("self_harm"),
        None,
    )];
    scan_response.mutation = Some(true);
    scan_response.ipi = Some(false);
    let error = guardrail(OnFlaggedAction::Block)
        .process_response(&scan_response, GuardrailInputType::Response, &[], &[])
        .expect_err("blocked");
    let Some(GraySwanFailure::Blocked {
        violation_location,
        violation,
        violated_rules,
        mutation,
        ipi,
        ..
    }) = error.cause
    else {
        panic!("expected a Blocked cause");
    };
    assert_eq!(violation_location, "output");
    assert!((violation - 0.9).abs() < f64::EPSILON);
    assert_eq!(
        violated_rules,
        vec![GuardrailExtraValue::Object(BTreeMap::from([
            ("rule".to_owned(), GuardrailExtraValue::SignedInteger(6)),
            (
                "name".to_owned(),
                GuardrailExtraValue::Text("self_harm".to_owned())
            ),
        ]))]
    );
    assert_eq!(mutation, Some(true));
    assert_eq!(ipi, Some(false));
}

#[test]
fn grayswan_block_error_marks_a_pre_call_violation_as_the_input_location() {
    let error = guardrail(OnFlaggedAction::Block)
        .process_response(&response(0.9), GuardrailInputType::Request, &[], &[])
        .expect_err("blocked");
    let Some(GraySwanFailure::Blocked {
        violation_location, ..
    }) = error.cause
    else {
        panic!("expected a Blocked cause");
    };
    assert_eq!(violation_location, "input");
}

#[test]
fn grayswan_block_error_status_code_is_bad_request() {
    let error = guardrail(OnFlaggedAction::Block)
        .process_response(&response(0.9), GuardrailInputType::Request, &[], &[])
        .expect_err("blocked");
    assert_eq!(error.status_code, Some(StatusCode::BAD_REQUEST.as_u16()));
    assert_eq!(error.http_status(), StatusCode::BAD_REQUEST);
    assert!(error.message.starts_with(GRAYSWAN_BLOCK_ERROR_MSG));
}

fn connection_refused() -> reqwest::Error {
    let listener = StdTcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime
        .block_on(
            reqwest::Client::default()
                .get(format!("http://{addr}"))
                .send(),
        )
        .unwrap_err()
}

#[rstest]
#[case::timeout(
    GraySwanFailure::Timeout { source: connection_refused() },
    StatusCode::REQUEST_TIMEOUT,
    Some(408)
)]
#[case::http_status_in_range(
    GraySwanFailure::HttpStatus { status: StatusCode::NOT_FOUND },
    StatusCode::NOT_FOUND,
    Some(404)
)]
#[case::http_status_at_the_lower_bound(
    GraySwanFailure::HttpStatus { status: StatusCode::BAD_REQUEST },
    StatusCode::BAD_REQUEST,
    Some(400)
)]
#[case::http_status_at_the_upper_bound(
    GraySwanFailure::HttpStatus { status: StatusCode::from_u16(599).unwrap() },
    StatusCode::from_u16(599).unwrap(),
    Some(599)
)]
#[case::http_status_out_of_range(
    GraySwanFailure::HttpStatus { status: StatusCode::MOVED_PERMANENTLY },
    StatusCode::MOVED_PERMANENTLY,
    Some(301)
)]
#[case::blocked(
    GraySwanFailure::Blocked {
        reason: String::new(),
        violation_location: "input",
        violation: 0.9_f64,
        violated_rules: Vec::new(),
        mutation: None,
        ipi: None,
    },
    StatusCode::BAD_REQUEST,
    Some(400)
)]
#[case::request(
    GraySwanFailure::Request { source: connection_refused() },
    StatusCode::INTERNAL_SERVER_ERROR,
    None
)]
#[case::invalid_header(
    GraySwanFailure::InvalidHeader {
        source: HeaderValue::from_str("\u{0}").unwrap_err(),
    },
    StatusCode::INTERNAL_SERVER_ERROR,
    None
)]
#[case::response_too_large(
    GraySwanFailure::ResponseTooLarge { limit: 10 },
    StatusCode::INTERNAL_SERVER_ERROR,
    None
)]
#[case::response_json(
    GraySwanFailure::ResponseJson {
        source: serde_json::from_str::<()>("not json").unwrap_err(),
    },
    StatusCode::INTERNAL_SERVER_ERROR,
    None
)]
fn grayswan_error_status_follows_litellm(
    #[case] failure: GraySwanFailure,
    #[case] status: StatusCode,
    #[case] carried: Option<u16>,
) {
    assert_eq!(failure.statuses(), (status, carried));
    let error = GraySwanGuardrailAPIError::from_failure(failure);
    assert_eq!(error.status_code, carried);
    assert_eq!(error.http_status(), status);
}

#[test]
fn grayswan_api_error_literal_stores_the_status_code() {
    let error = GraySwanGuardrailAPIError {
        message: "boom".to_owned(),
        status_code: Some(429),
        cause: None,
    };
    assert_eq!(error.message, "boom");
    assert_eq!(error.status_code, Some(429));
    assert!(error.cause.is_none());
    assert_eq!(error.to_string(), "boom");
    assert_eq!(error.http_status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn grayswan_passthrough_replaces_post_call_messages() {
    let result = guardrail(OnFlaggedAction::Passthrough)
        .process_response(&response(0.5), GuardrailInputType::Response, &[], &[])
        .expect("post-call passthrough");
    assert_eq!(result.texts.len(), 1);
    assert!(matches!(result.verdict, GraySwanVerdict::Replaced { .. }));
}

#[test]
fn grayswan_passthrough_short_circuits_pre_call_instead_of_replacing_messages() {
    let original = vec!["hello".to_owned()];
    let result = guardrail(OnFlaggedAction::Passthrough)
        .process_response(&response(0.5), GuardrailInputType::Request, &original, &[])
        .expect("pre-call passthrough short-circuits instead of erroring");
    assert_eq!(result.texts, original);
    assert!(matches!(
        result.verdict,
        GraySwanVerdict::ShortCircuited { .. }
    ));
}

fn violation_message(verdict: &GraySwanVerdict) -> Option<&str> {
    match verdict {
        GraySwanVerdict::Pass => None,
        GraySwanVerdict::Monitored { violation_message }
        | GraySwanVerdict::Replaced { violation_message }
        | GraySwanVerdict::ShortCircuited { violation_message } => Some(violation_message),
    }
}

fn structured_rule(
    rule: Option<GraySwanRuleNumber>,
    name: Option<&str>,
    description: Option<&str>,
) -> GraySwanViolatedRule {
    GraySwanViolatedRule::Structured(GraySwanViolatedRuleFields {
        rule,
        name: name.map(ToOwned::to_owned),
        description: description.map(ToOwned::to_owned),
        extra: BTreeMap::new(),
    })
}

#[test]
fn grayswan_verdict_message_includes_rule_name_and_description() {
    let mut scan_response = response(0.9);
    scan_response.violated_rule_descriptions = vec![structured_rule(
        None,
        Some("self_harm"),
        Some("content promotes self-harm"),
    )];
    let result = guardrail(OnFlaggedAction::Passthrough)
        .process_response(&scan_response, GuardrailInputType::Response, &[], &[])
        .expect("post-call passthrough");
    assert_eq!(
        violation_message(&result.verdict),
        Some(
            "Gray Swan Cygnal flagged the model response with a violation score of 0.90. \
             It violated rule(s): self_harm: content promotes self-harm."
        )
    );
}

#[test]
fn grayswan_verdict_message_includes_rule_name_only_when_description_missing() {
    let mut scan_response = response(0.9);
    scan_response.violated_rule_descriptions = vec![structured_rule(None, Some("self_harm"), None)];
    let result = guardrail(OnFlaggedAction::Passthrough)
        .process_response(&scan_response, GuardrailInputType::Response, &[], &[])
        .expect("post-call passthrough");
    assert_eq!(
        violation_message(&result.verdict),
        Some(
            "Gray Swan Cygnal flagged the model response with a violation score of 0.90. \
             It violated rule(s): self_harm."
        )
    );
}

#[test]
fn grayswan_verdict_message_prefixes_the_rule_number_when_present() {
    let mut scan_response = response(0.9);
    scan_response.violated_rule_descriptions = vec![structured_rule(
        Some(GraySwanRuleNumber::Integer(6)),
        Some("Illegal Activities"),
        Some("content promotes illegal activities"),
    )];
    let result = guardrail(OnFlaggedAction::Passthrough)
        .process_response(&scan_response, GuardrailInputType::Response, &[], &[])
        .expect("post-call passthrough");
    assert_eq!(
        violation_message(&result.verdict),
        Some(
            "Gray Swan Cygnal flagged the model response with a violation score of 0.90. \
             It violated rule(s): #6 Illegal Activities: content promotes illegal activities."
        )
    );
}

#[test]
fn grayswan_verdict_message_prefixes_an_explicit_zero_rule_number() {
    let mut scan_response = response(0.9);
    scan_response.violated_rule_descriptions = vec![structured_rule(
        Some(GraySwanRuleNumber::Integer(0)),
        Some("Illegal Activities"),
        Some("content promotes illegal activities"),
    )];
    let result = guardrail(OnFlaggedAction::Passthrough)
        .process_response(&scan_response, GuardrailInputType::Response, &[], &[])
        .expect("post-call passthrough");
    assert_eq!(
        violation_message(&result.verdict),
        Some(
            "Gray Swan Cygnal flagged the model response with a violation score of 0.90. \
             It violated rule(s): #0 Illegal Activities: content promotes illegal activities."
        )
    );
}

#[test]
fn grayswan_verdict_message_prefixes_an_explicit_empty_string_rule_number() {
    let mut scan_response = response(0.9);
    scan_response.violated_rule_descriptions = vec![structured_rule(
        Some(GraySwanRuleNumber::Text(String::new())),
        Some("Illegal Activities"),
        Some("content promotes illegal activities"),
    )];
    let result = guardrail(OnFlaggedAction::Passthrough)
        .process_response(&scan_response, GuardrailInputType::Response, &[], &[])
        .expect("post-call passthrough");
    assert_eq!(
        violation_message(&result.verdict),
        Some(
            "Gray Swan Cygnal flagged the model response with a violation score of 0.90. \
             It violated rule(s): # Illegal Activities: content promotes illegal activities."
        )
    );
}

#[test]
fn grayswan_verdict_message_falls_back_to_a_typed_plain_value_for_an_unrecognized_rule_shape() {
    let mut scan_response = response(0.9);
    scan_response.violated_rule_descriptions = vec![GraySwanViolatedRule::Plain(
        GuardrailExtraValue::Text("profanity".to_owned()),
    )];
    let result = guardrail(OnFlaggedAction::Passthrough)
        .process_response(&scan_response, GuardrailInputType::Response, &[], &[])
        .expect("post-call passthrough");
    assert_eq!(
        violation_message(&result.verdict),
        Some(
            "Gray Swan Cygnal flagged the model response with a violation score of 0.90. \
             It violated rule(s): profanity."
        )
    );
}

#[test]
fn grayswan_verdict_message_falls_back_to_typed_fields_for_a_nameless_rule_object() {
    let mut scan_response = response(0.9);
    scan_response.violated_rule_descriptions = vec![structured_rule(
        Some(GraySwanRuleNumber::Integer(6)),
        None,
        Some("content promotes illegal activities"),
    )];
    let result = guardrail(OnFlaggedAction::Passthrough)
        .process_response(&scan_response, GuardrailInputType::Response, &[], &[])
        .expect("post-call passthrough");
    assert_eq!(
        violation_message(&result.verdict),
        Some(
            "Gray Swan Cygnal flagged the model response with a violation score of 0.90. \
             It violated rule(s): {description: content promotes illegal activities, rule: 6}."
        )
    );
}

#[test]
fn grayswan_monitor_verdict_passes_through() {
    let result = guardrail(OnFlaggedAction::Monitor)
        .process_response(&response(0.5), GuardrailInputType::Request, &[], &[])
        .expect("monitor action");
    assert!(matches!(result.verdict, GraySwanVerdict::Monitored { .. }));
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn subscriber(
    writer: Captured,
    level: tracing::Level,
) -> impl tracing::Subscriber + Send + Sync + 'static {
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_writer(move || writer.clone())
        .finish()
}

#[test]
fn grayswan_monitor_verdict_logs_the_flagged_content() {
    let captured = Captured::default();
    let result = with_default(subscriber(captured.clone(), tracing::Level::INFO), || {
        guardrail(OnFlaggedAction::Monitor).process_response(
            &response(0.5),
            GuardrailInputType::Request,
            &[],
            &[],
        )
    })
    .expect("monitor action");
    assert!(matches!(result.verdict, GraySwanVerdict::Monitored { .. }));
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(output.contains("Monitoring mode - allowing flagged content"));
}

#[test]
fn grayswan_error_policy_is_fail_open_by_default() {
    assert!(guardrail(OnFlaggedAction::Monitor).settings.fail_open);
}

#[test]
fn grayswan_below_threshold_passes_through_even_when_the_action_is_block() {
    let result = guardrail(OnFlaggedAction::Block)
        .process_response(&response(0.1), GuardrailInputType::Request, &[], &[])
        .expect("below threshold is not blocked");
    assert_eq!(result.verdict, GraySwanVerdict::Pass);
}

#[test]
fn grayswan_response_parses_a_numeric_violation_string() {
    let parsed: GraySwanResponse = serde_json::from_str(r#"{"violation": "0.9"}"#).unwrap();
    assert!((parsed.violation - 0.9).abs() < f64::EPSILON);
}

#[test]
fn grayswan_response_parses_a_plain_numeric_violation() {
    let bytes = br#"{"violation": 0.9}"#;
    let parsed: GraySwanResponse = serde_json::from_slice(bytes).unwrap();
    assert!((parsed.violation - 0.9).abs() < f64::EPSILON);
}

#[test]
fn grayswan_response_defaults_a_null_violation_to_zero() {
    let parsed: GraySwanResponse = serde_json::from_str(r#"{"violation": null}"#).unwrap();
    assert!((parsed.violation - 0.0).abs() < f64::EPSILON);
}

#[test]
fn grayswan_response_parses_an_empty_violation_string_as_zero() {
    let parsed: GraySwanResponse = serde_json::from_str(r#"{"violation": ""}"#).unwrap();
    assert!((parsed.violation - 0.0).abs() < f64::EPSILON);
}

#[test]
fn grayswan_response_defaults_a_missing_violation_to_zero() {
    let parsed: GraySwanResponse = serde_json::from_str("{}").unwrap();
    assert!((parsed.violation - 0.0).abs() < f64::EPSILON);
}

#[test]
fn grayswan_response_rejects_a_non_numeric_violation_string() {
    let error = serde_json::from_str::<GraySwanResponse>(r#"{"violation": "not-a-number"}"#)
        .expect_err("invalid violation text is rejected");
    assert!(error.to_string().contains("invalid violation score"));
}

#[test]
fn grayswan_response_with_a_numeric_string_violation_is_blocked_instead_of_passed_through() {
    let parsed: GraySwanResponse = serde_json::from_str(r#"{"violation": "0.9"}"#).unwrap();
    let error = guardrail(OnFlaggedAction::Block)
        .process_response(&parsed, GuardrailInputType::Request, &[], &[])
        .expect_err("a normalized violation above the threshold is blocked");
    assert!(matches!(error.cause, Some(GraySwanFailure::Blocked { .. })));
}

#[test]
fn replays_the_grayswan_response_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/grayswan_response");
    let paths: Vec<_> = fs::read_dir(corpus).unwrap().collect();
    assert!(!paths.is_empty());
    assert!(paths.into_iter().all(|entry| {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        serde_json::from_slice::<GraySwanResponse>(&bytes).is_ok()
    }));
}

#[test]
fn grayswan_supported_event_hooks_are_pre_during_post() {
    assert_eq!(
        GraySwanGuardrail::get_supported_event_hooks(),
        vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::DuringCall,
            GuardrailEventHooks::PostCall
        ]
    );
}

#[rstest]
#[case::pre_call(GuardrailEventHooks::PreCall, "pre_call")]
#[case::during_call(GuardrailEventHooks::DuringCall, "during_call")]
#[case::post_call(GuardrailEventHooks::PostCall, "post_call")]
#[case::logging_only(GuardrailEventHooks::LoggingOnly, "logging_only")]
#[case::pre_mcp(GuardrailEventHooks::PreMcpCall, "pre_mcp_call")]
#[case::during_mcp(GuardrailEventHooks::DuringMcpCall, "during_mcp_call")]
#[case::post_mcp(GuardrailEventHooks::PostMcpCall, "post_mcp_call")]
#[case::realtime(
    GuardrailEventHooks::RealtimeInputTranscription,
    "realtime_input_transcription"
)]
fn grayswan_hook_names_are_lowercase(
    #[case] event_type: GuardrailEventHooks,
    #[case] expected: &str,
) {
    assert_eq!(hook_name(event_type), expected);
}

include!("tests/constructor.rs");
