use std::sync::Arc;

use betterlite_logging::callback_logger::CallbackLogger as _;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::otel::legacy_lifecycle::logger::LegacyOtelLogger;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use opentelemetry::KeyValue;
use opentelemetry::trace::Status;
use opentelemetry_sdk::trace::SpanData;

use crate::support::{
    END, PROXY, Rig, START, attribute, in_memory_config, instant, text_attribute,
};

const PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.0,
    "model": "gpt-4o",
    "model_map_information": {},
    "messages": [{"role": "user", "content": "secret request"}],
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {}
}"#;

fn event(id: &str) -> LoggerEvent {
    let mut payload: StandardLoggingPayload = serde_json::from_str(PAYLOAD).expect("payload");
    id.clone_into(&mut payload.id);
    LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    }
}

fn has_secret(span: &SpanData) -> bool {
    span.attributes
        .iter()
        .any(|attribute| format!("{:?}", attribute.value).contains("secret request"))
}

#[test]
fn callback_logger_exposes_its_name_and_logging_flags() {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.in_memory(None)));
    assert_eq!(logger.callback_name(), "opentelemetry");
    assert!(logger.message_logging());
    assert!(!logger.turn_off_message_logging());
    assert!(!logger.redacts_messages_itself());

    let named = LegacyOtelLogger::new(Arc::new(rig.in_memory(Some("arize"))));
    assert_eq!(named.callback_name(), "arize");
}

#[test]
fn callback_logger_success_event_writes_primary_and_raw_spans() {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.in_memory(None)));
    logger.log_success_event(&event("call-1")).expect("success");
    assert_eq!(rig.names(), vec!["litellm_request", "raw_gen_ai_request"]);
    let primary = rig.span("litellm_request");
    assert_eq!(primary.status, Status::Ok);
    assert_eq!(primary.start_time, instant(START));
    assert_eq!(primary.end_time, instant(END));
}

#[test]
fn callback_logger_failure_event_writes_error_span() {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.in_memory(None)));
    logger.log_failure_event(&event("call-1")).expect("failure");
    let spans = rig.spans();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name, "litellm_request");
    assert!(matches!(spans[0].status, Status::Error { .. }));
    assert!(spans[0].events.is_empty());
}

#[rstest::rstest]
#[case::recorded(r#"{"error_class":"ProviderError","error_message":"request failed","traceback":"provider stack"}"#, "ProviderError", "request failed", "provider stack")]
#[case::absent_fields("{}", "", "", ": \n")]
#[case::null_fields(
    r#"{"error_class":null,"error_message":null,"traceback":null}"#,
    "",
    "",
    ": \n"
)]
fn callback_logger_failure_records_exception_event(
    #[case] information: &str,
    #[case] type_name: &str,
    #[case] message: &str,
    #[case] stacktrace: &str,
) {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.in_memory(None)));
    let mut request = event("failed-call");
    Arc::make_mut(&mut request.payload).error_information =
        Some(serde_json::from_str(information).expect("error information"));

    logger.log_failure_event(&request).expect("failure");

    let span = rig.span("litellm_request");
    assert!(matches!(span.status, Status::Error { .. }));
    assert_eq!(span.events.len(), 1);
    let exception = &span.events[0];
    assert_eq!(exception.name, "exception");
    assert_eq!(
        exception.attributes,
        vec![
            KeyValue::new("exception.type", type_name.to_owned()),
            KeyValue::new("exception.message", message.to_owned()),
            KeyValue::new("exception.stacktrace", stacktrace.to_owned()),
            KeyValue::new("exception.escaped", "False"),
        ]
    );
}

#[test]
fn callback_logger_preserves_generation_and_proxy_metadata() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let proxy = rig.parent(&lifecycle, PROXY);
    let logger = LegacyOtelLogger::new(Arc::new(lifecycle));
    let mut request = event("metadata-call");
    request.litellm_metadata = Arc::new(
        serde_json::from_str(&format!(
            r#"{{"generation_name":"custom-generation","litellm_parent_otel_span":"00-{}-{}-01"}}"#,
            proxy.context.trace_id(),
            proxy.context.span_id()
        ))
        .expect("request metadata"),
    );
    let payload = Arc::make_mut(&mut request.payload);
    payload.metadata.user_api_key_team_id = Some("team-1".to_owned());
    payload.metadata.user_api_key_team_alias = Some("team-alias".to_owned());

    logger.log_success_event(&request).expect("success");

    let named: Vec<SpanData> = rig
        .spans()
        .into_iter()
        .filter(|span| span.name == "custom-generation")
        .collect();
    assert_eq!(named.len(), 2, "both the primary and raw span use the name");
    assert_eq!(named[0].parent_span_id, proxy.context.span_id());
    assert_eq!(named[1].parent_span_id, named[0].span_context.span_id());
    let closed_proxy = rig.span(PROXY);
    assert_eq!(closed_proxy.status, Status::Ok);
    assert_eq!(
        text_attribute(&closed_proxy, "metadata.user_api_key_team_id"),
        "team-1"
    );
    assert_eq!(
        text_attribute(&closed_proxy, "metadata.user_api_key_team_alias"),
        "team-alias"
    );
}

#[tokio::test]
async fn callback_logger_async_events_use_the_lifecycle_handlers() {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.in_memory(None)));
    logger
        .async_log_success_event(Arc::new(event("call-1")))
        .await
        .expect("async success");
    logger
        .async_log_failure_event(Arc::new(event("call-2")))
        .await
        .expect("async failure");
    assert_eq!(rig.names().len(), 3);
}

#[test]
fn callback_logger_sync_success_dedupes_repeat() {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.in_memory(None)));
    let request = event("call-1");
    logger.log_success_event(&request).expect("first");
    logger.log_success_event(&request).expect("repeat");
    assert_eq!(rig.names().len(), 2);
}

#[test]
fn callback_logger_sync_failure_dedupes_repeat() {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.in_memory(None)));
    let request = event("call-1");
    logger.log_failure_event(&request).expect("first");
    logger.log_failure_event(&request).expect("repeat");
    assert_eq!(rig.names(), vec!["litellm_request"]);
}

fn success_redacts(value: Json) {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.in_memory(None)));
    let mut request = event("call-redacted");
    request.dynamic_params = Arc::new(Json::Object(IndexMap::from([(
        "turn_off_message_logging".to_owned(),
        value,
    )])));
    logger.log_success_event(&request).expect("success");
    assert!(!has_secret(&rig.span("litellm_request")));
}

#[test]
fn callback_logger_bool_turn_off_redacts_span_content() {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.in_memory(None)));
    logger
        .log_success_event(&event("call-visible"))
        .expect("success");
    assert!(has_secret(&rig.span("litellm_request")));
    success_redacts(Json::Bool(true));
}

#[rstest::rstest]
#[case::streaming(true, "True")]
#[case::non_streaming(false, "False")]
fn callback_logger_preserves_request_attributes(#[case] stream: bool, #[case] expected: &str) {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.in_memory(None)));
    let mut request = event("request-attributes");
    request.stream = stream;
    let payload = Arc::make_mut(&mut request.payload);
    payload.custom_llm_provider = Some("openai".to_owned());
    payload.model_parameters = serde_json::from_str(
        r#"{"max_tokens":128,"temperature":0.5,"top_p":0.75,"user":"user-1","stream":null}"#,
    )
    .expect("model parameters");

    logger.log_success_event(&request).expect("success");

    let primary = rig.span("litellm_request");
    assert_eq!(text_attribute(&primary, "gen_ai.system"), "openai");
    assert_eq!(text_attribute(&primary, "llm.is_streaming"), expected);
    assert_eq!(
        attribute(&primary, "gen_ai.request.max_tokens"),
        Some(&opentelemetry::Value::I64(128))
    );
    assert_eq!(
        attribute(&primary, "gen_ai.request.temperature"),
        Some(&opentelemetry::Value::F64(0.5))
    );
    assert_eq!(
        attribute(&primary, "gen_ai.request.top_p"),
        Some(&opentelemetry::Value::F64(0.75))
    );
    assert_eq!(text_attribute(&primary, "llm.user"), "user-1");
}

#[test]
fn callback_logger_preserves_semconv_provider() {
    use betterlite_logging::otel::vocabulary::semconv::OTELSemconvCategory;

    let rig = Rig::new();
    let mut config = in_memory_config();
    config
        .semconv_stability_opt_in
        .insert(OTELSemconvCategory::GenAiLatestExperimental);
    let logger = LegacyOtelLogger::new(Arc::new(rig.with_options(config, None, None, false)));
    let mut request = event("semconv-provider");
    request.stream = true;
    Arc::make_mut(&mut request.payload).custom_llm_provider = Some("openai".to_owned());

    logger.log_success_event(&request).expect("success");

    let spans = rig.spans();
    assert_eq!(spans.len(), 1);
    assert_eq!(text_attribute(&spans[0], "gen_ai.provider.name"), "openai");
    assert_eq!(
        attribute(&spans[0], "gen_ai.request.stream"),
        Some(&opentelemetry::Value::Bool(true))
    );
}
