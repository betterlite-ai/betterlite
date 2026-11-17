#![cfg(test)]

use std::collections::BTreeMap;
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
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::{TypesafePassthroughInput, TypesafePassthroughLogger};

const RESULT: &str = "raw result";

const CATALOG: &[(&str, &str)] = &[
    (
        "typesafe/gpt-4o",
        r#"{"litellm_provider": "typesafe", "input_cost_per_token": 0.0000025, "output_cost_per_token": 0.00001}"#,
    ),
    (
        "typesafe/gpt-4o-mini",
        r#"{"litellm_provider": "typesafe", "input_cost_per_token": 0.0000005, "output_cost_per_token": 0.000002}"#,
    ),
    (
        "openrouter/gpt-4o",
        r#"{"litellm_provider": "openrouter", "input_cost_per_token": 0.000001, "output_cost_per_token": 0.000003}"#,
    ),
    (
        "typesafe/gpt-4o-aliased",
        r#"{"litellm_provider": "typesafe", "input_cost_per_token": 0.000003, "output_cost_per_token": 0.000012, "aliases": ["typesafe/gpt-4o-alias"]}"#,
    ),
];

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

fn catalog(entries: &[(&str, &str)]) -> Arc<ModelCatalog> {
    let mut cost_entries: BTreeMap<String, Json> = BTreeMap::new();
    for (key, body) in entries {
        cost_entries.insert((*key).to_owned(), json(body));
    }
    let bytes = serde_json::to_vec(&cost_entries).expect("the catalog serializes");
    Arc::new(ModelCatalog::parse(&bytes, UnknownFields::Reject).expect("the catalog parses"))
}

struct Fixture {
    logging_obj: StreamLoggingObject,
    response_body: Json,
    request_body: Json,
    provider: String,
    result: String,
    kwargs: PassThroughLoggingKwargs,
}

impl Fixture {
    fn new(provider: &str, response_body: &str, request_body: &str) -> Self {
        Self {
            logging_obj: logging_object(),
            response_body: json(response_body),
            request_body: json(request_body),
            provider: provider.to_owned(),
            result: RESULT.to_owned(),
            kwargs: PassThroughLoggingKwargs::default(),
        }
    }

    fn caller_kwargs(mut self, kwargs: PassThroughLoggingKwargs) -> Self {
        self.kwargs = kwargs;
        self
    }

    fn input(&self) -> TypesafePassthroughInput<'_> {
        TypesafePassthroughInput {
            logging_obj: &self.logging_obj,
            response_body: &self.response_body,
            request_body: &self.request_body,
            custom_llm_provider: &self.provider,
            url_route: "https://api.typesafe.ai/v1/chat",
            result: &self.result,
            start_time: start(),
            end_time: end(),
            cache_hit: false,
            kwargs: self.kwargs.clone(),
        }
    }
}

fn run(fixture: &Fixture) -> HandlerOutcome {
    run_with(fixture, CATALOG)
}

fn run_with(fixture: &Fixture, entries: &[(&str, &str)]) -> HandlerOutcome {
    let input = fixture.input();
    TypesafePassthroughLogger::new(payload_builder(), catalog(entries))
        .typesafe_passthrough_handler(&input)
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

#[test]
fn typesafe_prices_the_response_model_from_the_registry() {
    let fixture = Fixture::new(
        "typesafe",
        r#"{"model": "gpt-4o", "usage": {"input_tokens": 100, "output_tokens": 20}}"#,
        r#"{"model": "gpt-4o-mini"}"#,
    );
    let outcome = run(&fixture);
    let payload = payload(&outcome);
    let call_details = details(&fixture.logging_obj);

    assert_eq!(text(field(&payload, "model")), "typesafe/gpt-4o");
    assert_eq!(text(field(&payload, "custom_llm_provider")), "typesafe");
    assert_eq!(
        decimal(field(&payload, "response_cost")),
        Decimal::new(45, 5)
    );
    assert_eq!(
        decimal(entry(&outcome, "response_cost")),
        Decimal::new(45, 5)
    );
    assert_eq!(decimal(&call_details["response_cost"]), Decimal::new(45, 5));
    assert_eq!(
        text(field(&payload, "custom_llm_provider")),
        text(entry(&outcome, "custom_llm_provider"))
    );
    assert_eq!(logged_model(&fixture.logging_obj), "typesafe/gpt-4o");
}

#[test]
fn typesafe_prices_the_request_model_when_the_response_names_none() {
    let fixture = Fixture::new(
        "typesafe",
        r#"{"usage": {"input_tokens": 1, "output_tokens": 2}}"#,
        r#"{"model": "gpt-4o"}"#,
    );
    let outcome = run(&fixture);
    let payload = payload(&outcome);

    assert_eq!(text(field(&payload, "model")), "typesafe/gpt-4o");
    assert_eq!(
        decimal(field(&payload, "response_cost")),
        Decimal::new(225, 7)
    );
}

#[test]
fn typesafe_falls_back_to_the_request_model_price_when_the_response_model_is_unpriced() {
    let fixture = Fixture::new(
        "typesafe",
        r#"{"model": "gpt-4o-mini-unpriced", "usage": {"input_tokens": 10, "output_tokens": 0}}"#,
        r#"{"model": "gpt-4o"}"#,
    );
    let outcome = run(&fixture);
    let payload = payload(&outcome);

    assert_eq!(
        text(field(&payload, "model")),
        "typesafe/gpt-4o-mini-unpriced"
    );
    assert_eq!(
        decimal(field(&payload, "response_cost")),
        Decimal::new(25, 6)
    );
}

#[rstest]
#[case::no_models("{}", "{}", "typesafe/unknown")]
#[case::empty_models(
    r#"{"model": "", "usage": {"input_tokens": 0, "output_tokens": 0}}"#,
    r#"{"model": ""}"#,
    "typesafe/unknown"
)]
#[case::non_string_model(
    r#"{"model": 7, "usage": {"input_tokens": 100, "output_tokens": 20}}"#,
    r#"{"model": "gpt-4o"}"#,
    "typesafe/gpt-4o"
)]
#[case::non_integer_tokens(
    r#"{"model": "gpt-4o", "usage": {"input_tokens": "many", "output_tokens": 20}}"#,
    r#"{"model": "gpt-4o"}"#,
    "typesafe/gpt-4o"
)]
#[case::numeric_string_tokens(
    r#"{"model": "gpt-4o", "usage": {"input_tokens": "100", "output_tokens": 20}}"#,
    r#"{"model": "gpt-4o"}"#,
    "typesafe/gpt-4o"
)]
fn typesafe_reads_a_failed_validation_as_empty_defaults(
    #[case] response_body: &str,
    #[case] request_body: &str,
    #[case] model: &str,
) {
    let fixture = Fixture::new("typesafe", response_body, request_body);
    let outcome = run(&fixture);
    let payload = payload(&outcome);

    assert_eq!(text(field(&payload, "model")), model);
    assert_eq!(decimal(field(&payload, "response_cost")), Decimal::ZERO);
    assert_eq!(integer(field(&payload, "prompt_tokens")), 0);
    assert_eq!(integer(field(&payload, "completion_tokens")), 0);
    assert_eq!(integer(field(&payload, "total_tokens")), 0);
    assert_eq!(text(field(&result_json(&outcome), "response")), RESULT);
}

#[test]
fn typesafe_takes_the_validation_failure_without_a_log_line() {
    let (logs, _guard) = capture_logs(tracing::Level::WARN);
    let fixture = Fixture::new(
        "typesafe",
        r#"{"model": "gpt-4o", "usage": {"input_tokens": "many"}}"#,
        "{}",
    );
    let outcome = run(&fixture);

    assert_eq!(text(field(&payload(&outcome), "model")), "typesafe/unknown");
    assert!(logs.contents().is_empty());
}

#[test]
fn typesafe_honors_the_openrouter_cost() {
    let fixture = Fixture::new(
        "openrouter",
        r#"{"model": "gpt-4o", "usage": {"input_tokens": 10, "output_tokens": 5, "cost": 0.42}}"#,
        "{}",
    );
    let outcome = run(&fixture);
    let payload = payload(&outcome);

    assert_eq!(text(field(&payload, "model")), "openrouter/gpt-4o");
    assert_eq!(text(field(&payload, "custom_llm_provider")), "openrouter");
    assert_eq!(
        decimal(field(&payload, "response_cost")),
        Decimal::new(42, 2)
    );
    assert_eq!(
        decimal(entry(&outcome, "response_cost")),
        Decimal::new(42, 2)
    );
}

#[test]
fn typesafe_honors_a_zero_openrouter_cost() {
    let fixture = Fixture::new(
        "openrouter",
        r#"{"model": "gpt-4o", "usage": {"input_tokens": 100, "output_tokens": 20, "cost": 0}}"#,
        "{}",
    );
    let outcome = run(&fixture);
    let payload = payload(&outcome);

    assert_eq!(decimal(field(&payload, "response_cost")), Decimal::ZERO);
    assert_eq!(decimal(entry(&outcome, "response_cost")), Decimal::ZERO);
}

#[test]
fn typesafe_ignores_the_usage_cost_for_any_other_provider() {
    let fixture = Fixture::new(
        "typesafe",
        r#"{"model": "gpt-4o", "usage": {"input_tokens": 100, "output_tokens": 20, "cost": 0.42}}"#,
        "{}",
    );
    let outcome = run(&fixture);

    assert_eq!(
        decimal(field(&payload(&outcome), "response_cost")),
        Decimal::new(45, 5)
    );
}

#[rstest]
#[case::absent_cost(r#"{"input_tokens": 10, "output_tokens": 5}"#)]
#[case::null_cost(r#"{"input_tokens": 10, "output_tokens": 5, "cost": null}"#)]
fn typesafe_prices_openrouter_tokens_when_the_cost_is_missing(#[case] usage: &str) {
    let fixture = Fixture::new(
        "openrouter",
        &format!(r#"{{"model": "gpt-4o", "usage": {usage}}}"#),
        "{}",
    );
    let outcome = run(&fixture);

    assert_eq!(
        decimal(field(&payload(&outcome), "response_cost")),
        Decimal::new(25, 6)
    );
}

#[test]
fn typesafe_logs_a_chat_completion_envelope_as_the_response() {
    let fixture = Fixture::new(
        "typesafe",
        r#"{"model": "gpt-4o", "usage": {"input_tokens": 100, "output_tokens": 20}}"#,
        "{}",
    );
    let outcome = run(&fixture);
    let payload = payload(&outcome);
    let response = field(&payload, "response");
    let envelope_id = text(field(response, "id"));

    assert_eq!(envelope_id.len(), 45);
    assert!(envelope_id.starts_with("chatcmpl-"));
    assert_eq!(text(field(&payload, "id")), envelope_id);
    assert_eq!(integer(field(response, "created")), end().timestamp());
    assert_eq!(text(field(response, "model")), "typesafe/gpt-4o");
    assert_eq!(text(field(response, "object")), "chat.completion");
    assert!(matches!(field(response, "system_fingerprint"), Json::Null));
}

#[test]
fn typesafe_logs_one_default_chat_completion_choice() {
    let fixture = Fixture::new(
        "typesafe",
        r#"{"model": "gpt-4o", "usage": {"input_tokens": 100, "output_tokens": 20}}"#,
        "{}",
    );
    let outcome = run(&fixture);
    let payload = payload(&outcome);
    let response = field(&payload, "response");
    let Json::List(choices) = field(response, "choices") else {
        panic!("the choices field is a list");
    };

    assert_eq!(choices.len(), 1);
    let choice = &choices[0];
    assert_eq!(text(field(choice, "finish_reason")), "stop");
    assert_eq!(integer(field(choice, "index")), 0);
    let message = field(choice, "message");
    assert!(matches!(field(message, "content"), Json::Null));
    assert_eq!(text(field(message, "role")), "assistant");
    assert!(matches!(field(message, "tool_calls"), Json::Null));
    assert!(matches!(field(message, "function_call"), Json::Null));
}

#[test]
fn typesafe_reports_the_envelope_usage_in_the_payload_and_the_kwargs() {
    let fixture = Fixture::new(
        "typesafe",
        r#"{"model": "gpt-4o", "usage": {"input_tokens": 100, "output_tokens": 20}}"#,
        "{}",
    );
    let outcome = run(&fixture);
    let payload = payload(&outcome);
    let usage = field(field(&payload, "response"), "usage");

    assert_eq!(integer(field(usage, "prompt_tokens")), 100);
    assert_eq!(integer(field(usage, "completion_tokens")), 20);
    assert_eq!(integer(field(usage, "total_tokens")), 120);
    assert!(matches!(
        field(usage, "completion_tokens_details"),
        Json::Null
    ));
    assert!(matches!(field(usage, "prompt_tokens_details"), Json::Null));
    assert_eq!(integer(field(&payload, "prompt_tokens")), 100);
    assert_eq!(integer(field(&payload, "completion_tokens")), 20);
    assert_eq!(integer(field(&payload, "total_tokens")), 120);
    assert_eq!(
        integer(field(
            entry(&outcome, "combined_usage_object"),
            "total_tokens"
        )),
        120
    );
}

#[test]
fn typesafe_prices_a_model_through_its_catalog_alias() {
    let fixture = Fixture::new(
        "typesafe",
        r#"{"model": "gpt-4o-alias", "usage": {"input_tokens": 100, "output_tokens": 20}}"#,
        "{}",
    );
    let outcome = run(&fixture);
    let payload = payload(&outcome);

    assert_eq!(text(field(&payload, "model")), "typesafe/gpt-4o-alias");
    assert_eq!(
        decimal(field(&payload, "response_cost")),
        Decimal::new(54, 5)
    );
}

#[test]
fn typesafe_charges_nothing_when_no_catalog_key_names_the_model() {
    let fixture = Fixture::new(
        "typesafe",
        r#"{"usage": {"input_tokens": 100, "output_tokens": 20}}"#,
        "{}",
    );
    let outcome = run(&fixture);
    let payload = payload(&outcome);

    assert_eq!(text(field(&payload, "model")), "typesafe/unknown");
    assert_eq!(decimal(field(&payload, "response_cost")), Decimal::ZERO);
    assert_eq!(integer(field(&payload, "prompt_tokens")), 100);
    assert_eq!(integer(field(&payload, "completion_tokens")), 20);
}

#[test]
fn typesafe_reads_a_missing_input_rate_as_zero() {
    let fixture = Fixture::new(
        "typesafe",
        r#"{"model": "gpt-4o-output-only", "usage": {"input_tokens": 100, "output_tokens": 20}}"#,
        "{}",
    );
    let outcome = run_with(
        &fixture,
        &[(
            "typesafe/gpt-4o-output-only",
            r#"{"litellm_provider": "typesafe", "output_cost_per_token": 0.00001}"#,
        )],
    );

    assert_eq!(
        decimal(field(&payload(&outcome), "response_cost")),
        Decimal::new(2, 4)
    );
}

#[test]
fn typesafe_reads_a_missing_output_rate_as_zero() {
    let fixture = Fixture::new(
        "typesafe",
        r#"{"model": "gpt-4o-input-only", "usage": {"input_tokens": 100, "output_tokens": 20}}"#,
        "{}",
    );
    let outcome = run_with(
        &fixture,
        &[(
            "typesafe/gpt-4o-input-only",
            r#"{"litellm_provider": "typesafe", "input_cost_per_token": 0.0000025}"#,
        )],
    );

    assert_eq!(
        decimal(field(&payload(&outcome), "response_cost")),
        Decimal::new(25, 5)
    );
}

#[test]
fn typesafe_always_returns_the_relayed_result() {
    let fixture = Fixture::new("typesafe", "[]", "\"not an object\"")
        .caller_kwargs(PassThroughLoggingKwargs::default());
    let outcome = run(&fixture);
    let payload = payload(&outcome);

    assert_eq!(text(field(&result_json(&outcome), "response")), RESULT);
    assert_eq!(text(field(&payload, "model")), "typesafe/unknown");
    assert_eq!(decimal(field(&payload, "response_cost")), Decimal::ZERO);
    assert_eq!(text(field(&payload, "status")), "success");
}

#[test]
fn typesafe_reads_an_absent_provider_as_an_empty_one() {
    let fixture = Fixture::new("", r#"{"model": "gpt-4o"}"#, "{}");
    let outcome = run(&fixture);
    let payload = payload(&outcome);

    assert_eq!(text(field(&payload, "model")), "/gpt-4o");
    assert_eq!(text(field(&payload, "custom_llm_provider")), "");
    assert_eq!(decimal(field(&payload, "response_cost")), Decimal::ZERO);
}

#[test]
fn typesafe_writes_the_combined_usage_into_the_call_details() {
    let fixture = Fixture::new(
        "typesafe",
        r#"{"model": "gpt-4o", "usage": {"input_tokens": 7, "output_tokens": 3}}"#,
        "{}",
    );
    let outcome = run(&fixture);
    let call_details = details(&fixture.logging_obj);

    assert_eq!(text(entry(&outcome, "model")), "typesafe/gpt-4o");
    assert_eq!(text(&call_details["model"]), "typesafe/gpt-4o");
    assert_eq!(text(&call_details["custom_llm_provider"]), "typesafe");
    assert_eq!(
        integer(field(
            &call_details["combined_usage_object"],
            "total_tokens"
        )),
        10
    );
}

#[test]
fn typesafe_keeps_the_callers_kwargs() {
    let mut entries = IndexMap::new();
    entries.insert("user".to_owned(), Json::Text("abc".to_owned()));
    let fixture = Fixture::new(
        "typesafe",
        r#"{"model": "gpt-4o", "usage": {"input_tokens": 7, "output_tokens": 3}}"#,
        "{}",
    )
    .caller_kwargs(PassThroughLoggingKwargs { entries });
    let outcome = run(&fixture);

    assert_eq!(text(entry(&outcome, "user")), "abc");
    assert_eq!(text(entry(&outcome, "model")), "typesafe/gpt-4o");
    assert_eq!(text(entry(&outcome, "custom_llm_provider")), "typesafe");
    assert_eq!(
        decimal(entry(&outcome, "response_cost")),
        Decimal::new(475, 7)
    );
}
