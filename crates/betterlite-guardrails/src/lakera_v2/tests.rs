#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::IntoFuture as _;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap as AxumHeaderMap, StatusCode as AxumStatusCode, Uri};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_translation::LakeraOnFlagged;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::guardrail_initializers::lakera_v2::LakeraV2Args;
use crate::guardrail_message_scope::scoped_indices::{ScanScope, ScopeFlags};
use crate::guardrail_records::error::GuardrailSpanError;
use crate::guardrail_records::invocation::{GuardrailHookName, GuardrailInvocation};
use crate::guardrail_records::record::{GuardrailInvocationRecord, GuardrailSpanSink};
use crate::lakera_pii::wire::{BreakdownItem, Message};
use crate::lakera_v2::Data;
use crate::lakera_v2::advisory::{humanize_lakera_block_reasons, validate_advisory_config};
use crate::lakera_v2::error::LakeraV2Error;
use crate::lakera_v2::guardrail::{DEFAULT_API_BASE, LakeraV2Guardrail, SUPPORTED_EVENT_HOOKS};
use crate::lakera_v2::hooks::{HookContext, HookOutcome};
use crate::lakera_v2::scope::{
    COMBINED_REASON, MULTIMODAL_REASON, NO_PAYLOAD_REASON, NO_WRITE_BACK_REASON, NOT_A_LIST_REASON,
    build_lakera_inspection_messages, unmaskable_reason,
};
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

const ESCAPED_TEMPLATE_TEXT: &str = "Invalid advisory_system_message template: must include a real {reason} placeholder (not an escaped {{reason}}) so the LLM sees why the request was flagged.";
const UNKNOWN_FIELD_TEXT: &str = "Invalid advisory_system_message template: 'foo'. The template must be a valid str.format() string using only the {reason} placeholder.";
const PAYLOAD_TEXT: &str = "on_flagged='inject_system_message' requires payload=True and breakdown=True: advisory mode masks any detected PII before appending the advisory note, and that masking can only happen when Lakera's response carries both the violation breakdown and the payload location data. Without them, PII would be forwarded to the model unredacted.";

fn args() -> LakeraV2Args {
    LakeraV2Args {
        name: "lakera-v2".to_owned(),
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        api_base: None,
        api_key: None,
        project_id: None,
        payload: Some(true),
        breakdown: Some(true),
        metadata: None,
        dev_info: Some(true),
        on_flagged: Some(LakeraOnFlagged::Block),
        skip_system_message: None,
        skip_tool_message_in_guardrail: None,
        advisory_system_message: None,
        default_on: false,
        timeout: None,
    }
}

fn base(default_on: bool) -> GuardrailBase {
    GuardrailBase {
        settings: GuardrailSettings {
            guardrail_name: Some("lakera-v2".to_owned()),
            supported_event_hooks: Some(SUPPORTED_EVENT_HOOKS.to_vec()),
            event_hook: None,
            default_on,
            mask_request_content: false,
            mask_response_content: false,
            violation_message_template: None,
            end_session_after_n_fails: None,
            on_violation: None,
            realtime_violation_message: None,
            on_sensitive_data: None,
            sensitive_data_route_to_model: None,
            sticky_session_routing: true,
            run_in_parallel: false,
            scan_raw_request: false,
            only_scan_new_messages: false,
            timeout: None,
            switches: GuardrailSwitches::default(),
        },
        mode: None,
    }
}

fn build(base_url: &str, on_flagged: LakeraOnFlagged, template: Option<&str>) -> LakeraV2Guardrail {
    let mut args = args();
    args.api_base = Some(base_url.to_owned());
    args.api_key = Some(SecretString::from("k"));
    args.on_flagged = Some(on_flagged);
    args.advisory_system_message = template.map(str::to_owned);
    let env = BTreeMap::new();
    LakeraV2Guardrail::new(args, base(true), reqwest::Client::default(), &env).expect("built")
}

fn message(role: &str, content: &str) -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([
        ("role".to_owned(), GuardrailValue::Text(role.to_owned())),
        (
            "content".to_owned(),
            GuardrailValue::Text(content.to_owned()),
        ),
    ]))
}

fn chat_data() -> Data {
    let mut data = Data::new();
    data.insert(
        "model".to_owned(),
        GuardrailValue::Text("gpt-4o".to_owned()),
    );
    data.insert(
        "messages".to_owned(),
        GuardrailValue::Array(vec![message("user", "email a@example.com")]),
    );
    data
}

fn content_of(data: &Data, index: usize) -> Option<String> {
    let Some(GuardrailValue::Array(messages)) = data.get("messages") else {
        return None;
    };
    let Some(GuardrailValue::Object(fields)) = messages.get(index) else {
        return None;
    };
    match fields.get("content") {
        Some(GuardrailValue::Text(text)) => Some(text.clone()),
        _ => None,
    }
}

fn record_of(data: &Data) -> &GuardrailValue {
    let Some(GuardrailValue::Object(metadata)) = data.get("metadata") else {
        panic!("the record bucket is missing: {data:?}");
    };
    let Some(GuardrailValue::Array(entries)) =
        metadata.get("standard_logging_guardrail_information")
    else {
        panic!("the record list is missing: {data:?}");
    };
    entries.first().expect("one record")
}

fn record_field<'record>(record: &'record GuardrailValue, key: &str) -> &'record GuardrailValue {
    let GuardrailValue::Object(fields) = record else {
        panic!("the record is not an object: {record:?}");
    };
    fields.get(key).expect("the record field exists")
}

#[derive(Default)]
struct CollectingSink {
    records: Mutex<Vec<GuardrailInvocationRecord>>,
}

impl GuardrailSpanSink for CollectingSink {
    fn emit(&self, record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        self.records.lock().unwrap().push(record.clone());
        Ok(())
    }
}

#[derive(Debug, Default)]
struct Captured {
    path: String,
    headers: BTreeMap<String, String>,
    body: String,
}

struct Server {
    base_url: String,
    captured: Arc<Mutex<Captured>>,
    _tasks: JoinSet<Result<(), std::io::Error>>,
}

async fn serve(status: u16, body: String) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let captured = Arc::new(Mutex::new(Captured::default()));
    let sink = Arc::clone(&captured);
    let answer = move |uri: Uri, headers: AxumHeaderMap, bytes: Bytes| {
        let sink = Arc::clone(&sink);
        async move {
            let mut recorded = sink.lock().unwrap();
            recorded.path = uri.path().to_owned();
            recorded.headers = headers
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .to_str()
                        .ok()
                        .map(|text| (name.as_str().to_owned(), text.to_owned()))
                })
                .collect();
            recorded.body = String::from_utf8(bytes.to_vec()).unwrap();
            drop(recorded);
            (AxumStatusCode::from_u16(status).unwrap(), body)
        }
    };
    let router = Router::new().fallback(answer);
    let mut tasks: JoinSet<Result<(), std::io::Error>> = JoinSet::new();
    tasks.spawn(axum::serve(listener, router).into_future());
    Server {
        base_url,
        captured,
        _tasks: tasks,
    }
}

impl Server {
    fn captured(&self) -> String {
        self.captured.lock().unwrap().body.clone()
    }

    fn header(&self, name: &str) -> Option<String> {
        self.captured.lock().unwrap().headers.get(name).cloned()
    }
}

fn body_of(server: &Server) -> Data {
    serde_json::from_str(&server.captured()).expect("a JSON request body")
}

fn lakera_message(role: &str, content: &str) -> Message {
    let text = serde_json::json!({"role": role, "content": content}).to_string();
    serde_json::from_str(&text).expect("a message")
}

fn invocation(data: &Data) -> GuardrailInvocation {
    GuardrailInvocation::begin(GuardrailHookName::PreCall, data, None, SystemTime::now())
        .expect("an invocation")
}

fn context<'context>(
    scope: ScopeFlags,
    invocation: &'context mut GuardrailInvocation,
    sink: &'context CollectingSink,
    suppressed: &'context BTreeSet<String>,
) -> HookContext<'context> {
    HookContext {
        scope,
        suppressed,
        invocation,
        sink,
        log_target: None,
    }
}

fn flagged(breakdown: &str, payload: &str) -> String {
    format!(r#"{{"flagged":true,"breakdown":{breakdown},"payload":{payload}}}"#)
}

const PII_ONLY: &str = r#"[{"detected":true,"detector_type":"pii/email"}]"#;
const MIXED: &str = r#"[{"detected":true,"detector_type":"pii/email"},{"detected":true,"detector_type":"prompt_attack/jailbreak"}]"#;
const ATTACK_ONLY: &str = r#"[{"detected":true,"detector_type":"prompt_attack/jailbreak"}]"#;
const EMAIL_SPAN: &str = r#"[{"message_id":0,"start":6,"end":19,"detector_type":"pii/email"}]"#;

enum Hook {
    Pre,
    During,
    Post,
}

async fn run_hook(
    guardrail: &LakeraV2Guardrail,
    data: &mut Data,
    response: Option<&mut Data>,
    hook: Hook,
) -> Result<HookOutcome, GuardrailFailure> {
    let sink = CollectingSink::default();
    let suppressed = BTreeSet::new();
    let mut call = invocation(data);
    let mut call_context = context(ScopeFlags::default(), &mut call, &sink, &suppressed);
    match hook {
        Hook::Pre => guardrail.pre_call(data, &mut call_context).await,
        Hook::During => guardrail.during_call(data, &mut call_context).await,
        Hook::Post => {
            let Some(response) = response else {
                panic!("the post_call hook needs a response");
            };
            guardrail.post_call(data, response, &mut call_context).await
        }
    }
}

#[test]
fn constructor_applies_the_declared_defaults() {
    let mut args = args();
    args.payload = None;
    args.breakdown = None;
    args.dev_info = None;
    args.on_flagged = None;
    let env = BTreeMap::new();
    let guardrail =
        LakeraV2Guardrail::new(args, base(true), reqwest::Client::default(), &env).expect("built");
    assert_eq!(guardrail.api_base, DEFAULT_API_BASE);
    assert!(guardrail.flags.payload);
    assert!(guardrail.flags.breakdown);
    assert!(guardrail.flags.dev_info);
    assert_eq!(guardrail.on_flagged, LakeraOnFlagged::Block);
    assert!(guardrail.advisory_system_message.is_none());
}

#[test]
fn constructor_reads_the_environment_fallbacks() {
    let env = BTreeMap::from([
        ("LAKERA_API_KEY".to_owned(), "env-key".to_owned()),
        (
            "LAKERA_API_BASE".to_owned(),
            "https://env.example".to_owned(),
        ),
    ]);
    let guardrail = LakeraV2Guardrail::new(args(), base(true), reqwest::Client::default(), &env)
        .expect("built");
    assert_eq!(guardrail.api_key.expose_secret(), "env-key");
    assert_eq!(guardrail.api_base, "https://env.example");
}

#[test]
fn constructor_leaves_an_empty_environment_value_unset() {
    let env = BTreeMap::from([
        ("LAKERA_API_KEY".to_owned(), String::new()),
        ("LAKERA_API_BASE".to_owned(), String::new()),
    ]);
    let guardrail = LakeraV2Guardrail::new(args(), base(true), reqwest::Client::default(), &env)
        .expect("built");
    assert_eq!(guardrail.api_key.expose_secret(), "");
    assert_eq!(guardrail.api_base, DEFAULT_API_BASE);
}

#[test]
fn constructor_prefers_the_configured_connection() {
    let mut args = args();
    args.api_key = Some(SecretString::from("arg-key"));
    args.api_base = Some("https://arg.example".to_owned());
    let env = BTreeMap::from([
        ("LAKERA_API_KEY".to_owned(), "env-key".to_owned()),
        (
            "LAKERA_API_BASE".to_owned(),
            "https://env.example".to_owned(),
        ),
    ]);
    let guardrail =
        LakeraV2Guardrail::new(args, base(true), reqwest::Client::default(), &env).expect("built");
    assert_eq!(guardrail.api_key.expose_secret(), "arg-key");
    assert_eq!(guardrail.api_base, "https://arg.example");
}

#[test]
fn supported_event_hooks_are_the_three_lifecycle_hooks() {
    assert_eq!(
        SUPPORTED_EVENT_HOOKS,
        [
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::DuringCall,
            GuardrailEventHooks::PostCall
        ]
    );
}

#[test]
fn constructor_refuses_the_escaped_reason_template() {
    let error = validate_advisory_config(
        LakeraOnFlagged::InjectSystemMessage,
        Some("Watch out for {{reason}}"),
        Some(true),
        Some(true),
    )
    .expect_err("the escaped placeholder is not a field");
    assert_eq!(error.to_string(), ESCAPED_TEMPLATE_TEXT);
}

#[test]
fn constructor_refuses_a_template_with_an_unknown_field() {
    let error = validate_advisory_config(
        LakeraOnFlagged::InjectSystemMessage,
        Some("{reason} {foo}"),
        Some(true),
        Some(true),
    )
    .expect_err("the field has no value");
    assert_eq!(error.to_string(), UNKNOWN_FIELD_TEXT);
}

#[test]
fn constructor_refuses_inject_without_payload_and_breakdown() {
    let error = validate_advisory_config(
        LakeraOnFlagged::InjectSystemMessage,
        None,
        Some(false),
        Some(true),
    )
    .expect_err("advisory mode needs the breakdown and payload");
    assert_eq!(error.to_string(), PAYLOAD_TEXT);
    let null_payload =
        validate_advisory_config(LakeraOnFlagged::InjectSystemMessage, None, None, Some(true))
            .expect_err("a null payload counts as off");
    assert_eq!(null_payload.to_string(), PAYLOAD_TEXT);
}

#[test]
fn constructor_accepts_a_valid_reason_template() {
    for template in [
        "Flagged for {reason}.",
        "{reason:>30}",
        "{{escaped}} {reason:.4}",
        "{reason!s}",
    ] {
        validate_advisory_config(
            LakeraOnFlagged::InjectSystemMessage,
            Some(template),
            Some(true),
            Some(true),
        )
        .unwrap_or_else(|error| panic!("{template} was refused: {error}"));
    }
}

#[test]
fn constructor_refuses_a_conversion_the_format_engine_lacks() {
    let error = validate_advisory_config(
        LakeraOnFlagged::InjectSystemMessage,
        Some("{reason!r}"),
        Some(true),
        Some(true),
    )
    .expect_err("a repr conversion has no Rust form here");
    assert!(
        error.to_string().starts_with(
            "Invalid advisory_system_message template: Unknown conversion specifier r."
        ),
        "{error}"
    );
}

#[test]
fn constructor_refuses_a_negative_timeout() {
    let mut args = args();
    args.timeout = Some(-1.0);
    let env = BTreeMap::new();
    let error = LakeraV2Guardrail::new(args, base(true), reqwest::Client::default(), &env)
        .expect_err("a negative timeout has no duration");
    assert!(matches!(error, LakeraV2Error::Timeout { .. }), "{error}");
}

#[test]
fn build_advisory_message_formats_the_template_with_the_humanized_reason() {
    let guardrail = build(
        "http://unused",
        LakeraOnFlagged::InjectSystemMessage,
        Some("Flagged: {reason}!"),
    );
    let response: crate::lakera_pii::wire::LakeraResponse = serde_json::from_str(&flagged(
        r#"[{"detected":true,"detector_type":"pii/email"},{"detected":true,"detector_type":"prompt_attack/x"}]"#,
        "[]",
    ))
    .unwrap();
    let message = guardrail.build_advisory_message(Some(&response)).unwrap();
    assert_eq!(
        message,
        "Flagged: personally identifiable information, a potential prompt injection attempt!"
    );
}

#[rstest]
#[case::prompt_injection("prompt_injection/x", "a potential prompt injection attempt")]
#[case::prompt_attack("prompt_attack/y", "a potential prompt injection attempt")]
#[case::pii("pii/email", "personally identifiable information")]
#[case::moderated("moderated_content/hate", "policy-violating content")]
#[case::unknown("custom_detector/one", "custom detector")]
fn humanize_maps_each_detector_prefix(#[case] detector: &str, #[case] expected: &str) {
    let item = BreakdownItem {
        detected: Some(true),
        detector_type: Some(detector.to_owned()),
        extra: BTreeMap::new(),
    };
    assert_eq!(humanize_lakera_block_reasons(Some(&[item])), expected);
}

#[test]
fn humanize_dedupes_and_keeps_first_seen_order() {
    let items = vec![
        BreakdownItem {
            detected: Some(true),
            detector_type: Some("pii/email".to_owned()),
            extra: BTreeMap::new(),
        },
        BreakdownItem {
            detected: Some(true),
            detector_type: Some("prompt_injection/x".to_owned()),
            extra: BTreeMap::new(),
        },
        BreakdownItem {
            detected: Some(false),
            detector_type: Some("moderated_content/hate".to_owned()),
            extra: BTreeMap::new(),
        },
        BreakdownItem {
            detected: Some(true),
            detector_type: Some("prompt_attack/y".to_owned()),
            extra: BTreeMap::new(),
        },
        BreakdownItem {
            detected: Some(true),
            detector_type: Some("pii/phone".to_owned()),
            extra: BTreeMap::new(),
        },
    ];
    assert_eq!(
        humanize_lakera_block_reasons(Some(&items)),
        "personally identifiable information, a potential prompt injection attempt"
    );
}

#[test]
fn humanize_answers_the_generic_phrase_without_detections() {
    assert_eq!(
        humanize_lakera_block_reasons(None),
        "a content safety concern"
    );
    assert_eq!(
        humanize_lakera_block_reasons(Some(&[])),
        "a content safety concern"
    );
    let undetected = BreakdownItem {
        detected: Some(false),
        detector_type: Some("pii/email".to_owned()),
        extra: BTreeMap::new(),
    };
    assert_eq!(
        humanize_lakera_block_reasons(Some(&[undetected])),
        "a content safety concern"
    );
    let empty_type = BreakdownItem {
        detected: Some(true),
        detector_type: Some(String::new()),
        extra: BTreeMap::new(),
    };
    assert_eq!(
        humanize_lakera_block_reasons(Some(&[empty_type])),
        "a content safety concern"
    );
}

fn response_with(breakdown: &str, payload: &str) -> crate::lakera_pii::wire::LakeraResponse {
    serde_json::from_str(&flagged(breakdown, payload)).expect("a response")
}

#[test]
fn unmaskable_reason_reports_the_first_obstacle_in_order() {
    let response = response_with(PII_ONLY, EMAIL_SPAN);
    let mut multimodal = chat_data();
    multimodal.insert(
        "messages".to_owned(),
        GuardrailValue::Array(vec![GuardrailValue::Object(BTreeMap::from([
            ("role".to_owned(), GuardrailValue::Text("user".to_owned())),
            (
                "content".to_owned(),
                GuardrailValue::Array(vec![message("text", "hi")]),
            ),
        ]))]),
    );
    assert_eq!(
        unmaskable_reason(&multimodal, Some(&response), ScopeFlags::default()),
        Some(MULTIMODAL_REASON)
    );
    let mut combined = chat_data();
    combined.insert(
        "input".to_owned(),
        GuardrailValue::Text("also this".to_owned()),
    );
    assert_eq!(
        unmaskable_reason(&combined, Some(&response), ScopeFlags::default()),
        Some(COMBINED_REASON)
    );
    let mut not_a_list = chat_data();
    not_a_list.insert(
        "messages".to_owned(),
        GuardrailValue::Text("not messages".to_owned()),
    );
    assert_eq!(
        unmaskable_reason(&not_a_list, Some(&response), ScopeFlags::default()),
        Some(NOT_A_LIST_REASON)
    );
    assert_eq!(
        unmaskable_reason(&chat_data(), Some(&response), ScopeFlags::default()),
        Some(NO_WRITE_BACK_REASON)
    );
    let mut instructions = chat_data();
    instructions.insert(
        "instructions".to_owned(),
        GuardrailValue::Text("be brief".to_owned()),
    );
    let without_payload = response_with(PII_ONLY, "[]");
    assert_eq!(
        unmaskable_reason(&instructions, Some(&without_payload), ScopeFlags::default()),
        Some(NO_PAYLOAD_REASON)
    );
    assert_eq!(
        unmaskable_reason(&instructions, Some(&response), ScopeFlags::default()),
        None
    );
}

#[test]
fn unmaskable_reason_ignores_instructions_skipped_by_the_scope() {
    let mut instructions = chat_data();
    instructions.insert(
        "instructions".to_owned(),
        GuardrailValue::Text("be brief".to_owned()),
    );
    let skipped = ScopeFlags {
        skip_system_message: true,
        skip_tool_message: false,
        scan: ScanScope::AllMessages,
    };
    assert_eq!(
        unmaskable_reason(
            &instructions,
            Some(&response_with(PII_ONLY, EMAIL_SPAN)),
            skipped
        ),
        Some(NO_WRITE_BACK_REASON)
    );
}

#[test]
fn inspection_messages_put_the_instructions_first() {
    let mut data = chat_data();
    data.insert(
        "instructions".to_owned(),
        GuardrailValue::Text("Always answer in English.".to_owned()),
    );
    let messages = build_lakera_inspection_messages(&data);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[0].content, "Always answer in English.");
    assert_eq!(messages[1].role, "user");
    assert_eq!(messages[1].content, "email a@example.com");
}

#[test]
fn inspection_messages_leave_out_empty_instructions() {
    let mut data = chat_data();
    data.insert(
        "instructions".to_owned(),
        GuardrailValue::Text(String::new()),
    );
    let messages = build_lakera_inspection_messages(&data);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, "user");
}

#[tokio::test]
async fn call_v2_guard_posts_the_documented_body_and_header() {
    let server = serve(
        200,
        r#"{"flagged":false,"payload":[],"breakdown":[]}"#.to_owned(),
    )
    .await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::Block, None);
    let mut data = chat_data();
    let sink = CollectingSink::default();
    let suppressed = BTreeSet::new();
    let mut call = invocation(&data);
    let mut call_context = context(ScopeFlags::default(), &mut call, &sink, &suppressed);
    let messages = vec![lakera_message("user", "email a@example.com")];
    let (response, counts) = guardrail
        .call_v2_guard(
            &messages,
            &mut data,
            GuardrailEventHooks::PreCall,
            &mut call_context,
        )
        .await
        .expect("a decoded response");
    assert_eq!(response.flagged, Some(false));
    assert!(counts.is_empty());
    let body = body_of(&server);
    assert_eq!(server.header("authorization"), Some("Bearer k".to_owned()));
    assert!(matches!(
        body.get("messages"),
        Some(GuardrailValue::Array(entries)) if entries.len() == 1
    ));
    assert_eq!(body.get("project_id"), Some(&GuardrailValue::Null));
    assert_eq!(body.get("payload"), Some(&GuardrailValue::Boolean(true)));
    assert_eq!(body.get("breakdown"), Some(&GuardrailValue::Boolean(true)));
    assert_eq!(body.get("dev_info"), Some(&GuardrailValue::Boolean(true)));
    assert_eq!(body.get("metadata"), Some(&GuardrailValue::Null));
    let captured = server.captured.lock().unwrap();
    assert_eq!(captured.path, "/v2/guard");
}

#[tokio::test]
async fn call_v2_guard_records_success_without_the_payload() {
    let server = serve(200, flagged(PII_ONLY, EMAIL_SPAN)).await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::Block, None);
    let mut data = chat_data();
    let sink = CollectingSink::default();
    let suppressed = BTreeSet::new();
    let mut call = invocation(&data);
    let mut call_context = context(ScopeFlags::default(), &mut call, &sink, &suppressed);
    let messages = vec![lakera_message("user", "email a@example.com")];
    guardrail
        .call_v2_guard(
            &messages,
            &mut data,
            GuardrailEventHooks::PreCall,
            &mut call_context,
        )
        .await
        .expect("a decoded response");
    let record = record_of(&data);
    assert_eq!(
        record_field(record, "guardrail_status"),
        &GuardrailValue::Text("success".to_owned())
    );
    assert_eq!(
        record_field(record, "masked_entity_count"),
        &GuardrailValue::Object(BTreeMap::new())
    );
    let GuardrailValue::Object(recorded) = record_field(record, "guardrail_response") else {
        panic!("the recorded response is not an object");
    };
    assert!(recorded.get("payload").is_none(), "{recorded:?}");
    assert_eq!(
        recorded.get("flagged"),
        Some(&GuardrailValue::Boolean(true))
    );
    assert_eq!(sink.records.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn call_v2_guard_records_a_failed_response() {
    let server = serve(200, "not json".to_owned()).await;
    let guardrail = build(&server.base_url, LakeraOnFlagged::Block, None);
    let mut data = chat_data();
    let sink = CollectingSink::default();
    let suppressed = BTreeSet::new();
    let mut call = invocation(&data);
    let mut call_context = context(ScopeFlags::default(), &mut call, &sink, &suppressed);
    let messages = vec![lakera_message("user", "hello")];
    let error = guardrail
        .call_v2_guard(
            &messages,
            &mut data,
            GuardrailEventHooks::PreCall,
            &mut call_context,
        )
        .await
        .expect_err("the body does not decode");
    assert!(matches!(error, LakeraV2Error::Decode { .. }), "{error}");
    let record = record_of(&data);
    assert_eq!(
        record_field(record, "guardrail_status"),
        &GuardrailValue::Text("guardrail_failed_to_respond".to_owned())
    );
    assert_eq!(
        record_field(record, "guardrail_response"),
        &GuardrailValue::Text(error.to_string())
    );
    assert_eq!(sink.records.lock().unwrap().len(), 1);
}

mod hooks;
