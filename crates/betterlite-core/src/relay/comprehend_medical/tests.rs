#![cfg(test)]

use std::io;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::logging_privacy::Settings;
use betterlite_error::normalize::Normalizer;
use betterlite_logging::logging_metadata::base64::Base64Truncator;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::logging_success::payload_state::STANDARD_LOGGING_OBJECT;
use betterlite_logging::logging_success::ports::StandardPayloadBuilder;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_logging::payload::builder::{PayloadBuilder, PayloadBuilderConfig};
use betterlite_logging::payload::deps::{PayloadSettings, PayloadSources};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use http::{HeaderMap, HeaderValue};
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::{ComprehendMedicalInput, ComprehendMedicalLogger, supported_operations};

const RESULT: &str = "{\"Entities\": []}";

struct NoopDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoopDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

fn start() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00").expect("a valid timestamp")
}

fn end() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2024-01-01T00:00:02+00:00").expect("a valid timestamp")
}

fn logging_init() -> LoggingInit {
    LoggingInit {
        model: String::new(),
        call_type: "pass_through_endpoint".to_owned(),
        start_time: start(),
        litellm_call_id: "call-1".to_owned(),
        function_id: "function-1".to_owned(),
        messages: None,
        stream: Some(false),
        litellm_trace_id: Some("trace-1".to_owned()),
        kwargs: IndexMap::new(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    }
}

fn logging_object() -> StreamLoggingObject {
    let redactor = Redactor::new(16).expect("a valid redactor");
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    let logging = LoggingState::new(
        logging_init(),
        setup,
        &redactor,
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("a valid logging state");
    StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: CorrelationContext::default(),
        })),
        dispatch: Arc::new(NoopDispatch),
    }
}

fn payload_builder() -> Arc<dyn StandardPayloadBuilder> {
    Arc::new(PayloadBuilder::new(PayloadBuilderConfig {
        sources: Arc::new(PayloadSources::default()),
        settings: PayloadSettings::default(),
        redactor: Arc::new(Redactor::new(16).expect("a valid redactor")),
        normalizer: Arc::new(Normalizer::new().expect("a valid normalizer")),
        truncator: Base64Truncator::new(0).expect("a valid truncator"),
        privacy: Settings::default(),
    }))
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("a JSON literal")
}

struct Fixture {
    logging_obj: StreamLoggingObject,
    request_headers: HeaderMap,
    request_body: Json,
    result: String,
    kwargs: PassThroughLoggingKwargs,
}

impl Fixture {
    fn new(target: Option<&str>, request_body: &str) -> Self {
        let mut headers = HeaderMap::new();
        if let Some(target_header) = target {
            headers.insert(
                "x-amz-target",
                HeaderValue::from_str(target_header).expect("a valid header value"),
            );
        }
        Self {
            logging_obj: logging_object(),
            request_headers: headers,
            request_body: json(request_body),
            result: RESULT.to_owned(),
            kwargs: PassThroughLoggingKwargs::default(),
        }
    }

    fn caller_kwargs(mut self, kwargs: PassThroughLoggingKwargs) -> Self {
        self.kwargs = kwargs;
        self
    }

    fn input(&self) -> ComprehendMedicalInput<'_> {
        ComprehendMedicalInput {
            logging_obj: &self.logging_obj,
            request_headers: &self.request_headers,
            request_body: &self.request_body,
            url_route: "https://comprehendmedical.us-east-1.amazonaws.com",
            result: &self.result,
            start_time: start(),
            end_time: end(),
            cache_hit: false,
            kwargs: self.kwargs.clone(),
        }
    }
}

fn run(fixture: &Fixture) -> HandlerOutcome {
    ComprehendMedicalLogger::new(payload_builder()).comprehend_medical_handler(fixture.input())
}

fn payload(outcome: &HandlerOutcome) -> Json {
    outcome
        .kwargs
        .entries
        .get(STANDARD_LOGGING_OBJECT)
        .cloned()
        .expect("the outcome carries a standard logging object")
}

fn field<'value>(value: &'value Json, name: &str) -> &'value Json {
    let Json::Object(fields) = value else {
        panic!("the value is an object");
    };
    fields.get(name).expect("the field is present")
}

fn text(value: &Json) -> &str {
    let Json::Text(text) = value else {
        panic!("the value is a string");
    };
    text.as_str()
}

fn integer(value: &Json) -> i64 {
    let Json::Number(number) = value else {
        panic!("the value is a number");
    };
    number.as_i64().expect("a whole number")
}

fn decimal(value: &Json) -> Decimal {
    let Json::Number(number) = value else {
        panic!("the value is a number");
    };
    Decimal::from_str(&number.to_string()).expect("a decimal number")
}

fn result_json(outcome: &HandlerOutcome) -> Json {
    outcome
        .result
        .clone()
        .expect("the handler returned a result")
        .into_json()
}

fn details(logging_obj: &StreamLoggingObject) -> IndexMap<String, Json> {
    logging_obj
        .read(&mut |state| state.model_call_details.clone())
        .expect("the logging state is present")
}

fn logged_model(logging_obj: &StreamLoggingObject) -> String {
    logging_obj
        .read(&mut |state| state.model.clone())
        .expect("the logging state is present")
}

fn entry<'outcome>(outcome: &'outcome HandlerOutcome, name: &str) -> &'outcome Json {
    outcome
        .kwargs
        .entries
        .get(name)
        .expect("the caller's entry is present")
}

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        str::from_utf8(&bytes)
            .expect("the captured log lines are UTF-8")
            .to_owned()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

fn capture_logs(level: tracing::Level) -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(level)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

fn text_body(characters: usize) -> String {
    format!(r#"{{"Text": "{}"}}"#, "x".repeat(characters))
}

#[test]
fn comprehend_supported_operations_are_the_five_tariff_keys() {
    let operations: Vec<&str> = supported_operations().collect();

    assert_eq!(
        operations,
        vec![
            "DetectEntitiesV2",
            "DetectPHI",
            "InferICD10CM",
            "InferRxNorm",
            "InferSNOMEDCT"
        ]
    );
}

#[rstest]
#[case::prefixed(
    Some("ComprehendMedical_V20181030.DetectPHI"),
    "comprehendmedical/DetectPHI"
)]
#[case::last_component(
    Some("ComprehendMedical_V20181030.A.DetectEntitiesV2"),
    "comprehendmedical/DetectEntitiesV2"
)]
#[case::bare(Some("InferICD10CM"), "comprehendmedical/InferICD10CM")]
#[case::trailing_dot(Some("ComprehendMedical_V20181030."), "comprehendmedical/")]
#[case::empty(Some(""), "comprehendmedical/")]
#[case::missing(None, "comprehendmedical/")]
#[case::unknown(
    Some("ComprehendMedical_V20181030.DetectSentiment"),
    "comprehendmedical/DetectSentiment"
)]
fn comprehend_names_the_operation_from_the_amz_target_header(
    #[case] target: Option<&str>,
    #[case] model: &str,
) {
    let fixture = Fixture::new(target, r#"{"Text": "hello"}"#);
    let outcome = run(&fixture);
    let payload = payload(&outcome);

    assert_eq!(text(field(&payload, "model")), model);
    assert_eq!(text(entry(&outcome, "model")), model);
}

#[rstest]
#[case::empty_text("DetectEntitiesV2", 0, Decimal::new(1, 2))]
#[case::one_character("DetectEntitiesV2", 1, Decimal::new(1, 2))]
#[case::one_unit("DetectEntitiesV2", 100, Decimal::new(1, 2))]
#[case::second_unit("DetectEntitiesV2", 101, Decimal::new(2, 2))]
#[case::phi_unit("DetectPHI", 100, Decimal::new(14, 4))]
#[case::phi_second_unit("DetectPHI", 101, Decimal::new(28, 4))]
#[case::three_units("InferICD10CM", 250, Decimal::new(15, 4))]
#[case::two_units("InferRxNorm", 200, Decimal::new(5, 4))]
#[case::snomed_units("InferSNOMEDCT", 150, Decimal::new(15, 3))]
fn comprehend_charges_started_hundred_character_units(
    #[case] operation: &str,
    #[case] characters: usize,
    #[case] cost: Decimal,
) {
    let target = format!("ComprehendMedical_V20181030.{operation}");
    let fixture = Fixture::new(Some(&target), &text_body(characters));
    let outcome = run(&fixture);

    assert_eq!(decimal(field(&payload(&outcome), "response_cost")), cost);
    assert_eq!(decimal(entry(&outcome, "response_cost")), cost);
}

#[test]
fn comprehend_charges_nothing_for_an_unknown_operation_without_a_warning() {
    let (logs, _guard) = capture_logs(tracing::Level::WARN);
    let fixture = Fixture::new(
        Some("ComprehendMedical_V20181030.DetectSentiment"),
        &text_body(250),
    );
    let outcome = run(&fixture);

    assert_eq!(
        decimal(field(&payload(&outcome), "response_cost")),
        Decimal::ZERO
    );
    assert!(logs.contents().is_empty());
}

#[rstest]
#[case::absent("{}")]
#[case::not_a_string(r#"{"Text": 5}"#)]
#[case::null(r#"{"Text": null}"#)]
fn comprehend_reads_a_missing_text_as_one_unit(#[case] body: &str) {
    let fixture = Fixture::new(Some("ComprehendMedical_V20181030.DetectPHI"), body);
    let outcome = run(&fixture);

    assert_eq!(
        decimal(field(&payload(&outcome), "response_cost")),
        Decimal::new(14, 4)
    );
}

#[test]
fn comprehend_counts_characters_rather_than_bytes() {
    let body = format!(r#"{{"Text": "{}"}}"#, "\u{e9}".repeat(101));
    let fixture = Fixture::new(Some("ComprehendMedical_V20181030.DetectPHI"), &body);
    let outcome = run(&fixture);

    assert_eq!(
        decimal(field(&payload(&outcome), "response_cost")),
        Decimal::new(28, 4)
    );
}

#[test]
fn comprehend_writes_the_model_provider_cost_and_result() {
    let fixture = Fixture::new(
        Some("ComprehendMedical_V20181030.DetectPHI"),
        &text_body(100),
    );
    let outcome = run(&fixture);
    let payload = payload(&outcome);
    let call_details = details(&fixture.logging_obj);

    assert_eq!(
        text(field(&payload, "model")),
        "comprehendmedical/DetectPHI"
    );
    assert_eq!(
        text(field(&payload, "custom_llm_provider")),
        "comprehendmedical"
    );
    assert_eq!(text(field(field(&payload, "response"), "response")), RESULT);
    assert_eq!(text(field(&result_json(&outcome), "response")), RESULT);
    assert_eq!(integer(field(&payload, "total_tokens")), 0);
    assert_eq!(
        logged_model(&fixture.logging_obj),
        "comprehendmedical/DetectPHI"
    );
    assert_eq!(text(&call_details["model"]), "comprehendmedical/DetectPHI");
    assert_eq!(
        text(&call_details["custom_llm_provider"]),
        "comprehendmedical"
    );
    assert_eq!(decimal(&call_details["response_cost"]), Decimal::new(14, 4));
}

#[test]
fn comprehend_keeps_the_callers_kwargs() {
    let mut entries = IndexMap::new();
    entries.insert("user".to_owned(), Json::Text("abc".to_owned()));
    let fixture = Fixture::new(
        Some("ComprehendMedical_V20181030.DetectPHI"),
        &text_body(100),
    )
    .caller_kwargs(PassThroughLoggingKwargs { entries });
    let outcome = run(&fixture);

    assert_eq!(text(entry(&outcome, "user")), "abc");
    assert_eq!(
        text(entry(&outcome, "model")),
        "comprehendmedical/DetectPHI"
    );
    assert_eq!(
        text(entry(&outcome, "custom_llm_provider")),
        "comprehendmedical"
    );
    assert_eq!(
        decimal(entry(&outcome, "response_cost")),
        Decimal::new(14, 4)
    );
}

#[test]
fn comprehend_warns_and_returns_the_raw_result_for_a_body_that_is_not_an_object() {
    let (logs, _guard) = capture_logs(tracing::Level::ERROR);
    let mut entries = IndexMap::new();
    entries.insert("model".to_owned(), Json::Text("caller-model".to_owned()));
    let caller = PassThroughLoggingKwargs { entries };
    let fixture = Fixture::new(Some("ComprehendMedical_V20181030.DetectPHI"), "[]")
        .caller_kwargs(caller.clone());
    let outcome = run(&fixture);

    let contents = logs.contents();
    assert!(contents.contains("Error in Comprehend Medical passthrough logging handler"));
    assert!(contents.contains("must be a JSON object, found a list"));
    assert_eq!(outcome.kwargs, caller);
    assert_eq!(text(field(&result_json(&outcome), "response")), RESULT);
    let call_details = details(&fixture.logging_obj);
    assert!(logged_model(&fixture.logging_obj).is_empty());
    assert!(text(&call_details["model"]).is_empty());
    assert!(!call_details.contains_key("response_cost"));
}
