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
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::{CursorPassthroughInput, CursorPassthroughLogger, path_matches};

const RESULT: &str = "raw result";

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
    method: String,
    url_route: String,
    response_body: Json,
    request_body: Json,
    result: String,
    kwargs: PassThroughLoggingKwargs,
}

impl Fixture {
    fn new(method: &str, url_route: &str) -> Self {
        Self {
            logging_obj: logging_object(),
            method: method.to_owned(),
            url_route: url_route.to_owned(),
            response_body: Json::Object(IndexMap::new()),
            request_body: Json::Object(IndexMap::new()),
            result: RESULT.to_owned(),
            kwargs: PassThroughLoggingKwargs::default(),
        }
    }

    fn body(mut self, body: Json) -> Self {
        self.response_body = body;
        self
    }

    fn caller_kwargs(mut self, kwargs: PassThroughLoggingKwargs) -> Self {
        self.kwargs = kwargs;
        self
    }

    fn input(&self) -> CursorPassthroughInput<'_> {
        CursorPassthroughInput {
            logging_obj: &self.logging_obj,
            method: &self.method,
            url_route: &self.url_route,
            response_body: &self.response_body,
            request_body: &self.request_body,
            result: &self.result,
            start_time: start(),
            end_time: end(),
            cache_hit: false,
            kwargs: self.kwargs.clone(),
        }
    }
}

fn run(fixture: &Fixture) -> HandlerOutcome {
    CursorPassthroughLogger::new(payload_builder()).cursor_passthrough_handler(fixture.input())
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

#[rstest]
#[case::create_agent("POST", "/v0/agents", "cursor/cursor:agent:create")]
#[case::list_agents("GET", "/v0/agents", "cursor/cursor:agent:list")]
#[case::followup("POST", "/v0/agents/bc-1/followup", "cursor/cursor:agent:followup")]
#[case::stop("POST", "/v0/agents/bc-1/stop", "cursor/cursor:agent:stop")]
#[case::delete("DELETE", "/v0/agents/bc-1", "cursor/cursor:agent:delete")]
#[case::conversation(
    "GET",
    "/v0/agents/bc-1/conversation",
    "cursor/cursor:agent:conversation"
)]
#[case::status("GET", "/v0/agents/bc-1", "cursor/cursor:agent:status")]
#[case::account("GET", "/v0/me", "cursor/cursor:account:info")]
#[case::models("GET", "/v0/models", "cursor/cursor:models:list")]
#[case::repositories("GET", "/v0/repositories", "cursor/cursor:repositories:list")]
#[case::lowercase_method("get", "/v0/me", "cursor/cursor:account:info")]
#[case::trailing_slash("POST", "/v0/agents/", "cursor/cursor:agent:create")]
#[case::no_leading_slash("GET", "v0/me", "cursor/cursor:account:info")]
#[case::absolute_route(
    "GET",
    "https://api.cursor.com/v0/agents/bc-1",
    "cursor/cursor:agent:status"
)]
#[case::query_dropped("GET", "/v0/models?limit=5", "cursor/cursor:models:list")]
#[case::fragment_dropped("GET", "/v0/models#page", "cursor/cursor:models:list")]
#[case::dot_segments(
    "GET",
    "https://api.cursor.com/v0/agents/../me",
    "cursor/cursor:account:info"
)]
#[case::encoded_separator("GET", "/v0/agents%2Fbc-1", "cursor/cursor:agent:status")]
#[case::unknown_path("POST", "/v0/unknown", "cursor/cursor:post:/v0/unknown")]
#[case::wrong_method("PUT", "/v0/agents", "cursor/cursor:put:/v0/agents")]
#[case::unknown_subpath(
    "GET",
    "/v0/agents/bc-1/unknown",
    "cursor/cursor:get:/v0/agents/bc-1/unknown"
)]
#[case::root_path("GET", "/", "cursor/cursor:get:")]
#[case::empty_route("GET", "", "cursor/cursor:get:")]
#[case::trailing_slashes("GET", "/v0/agents2/", "cursor/cursor:get:/v0/agents2")]
fn cursor_names_the_operation_of_every_request(
    #[case] method: &str,
    #[case] route: &str,
    #[case] model: &str,
) {
    let fixture = Fixture::new(method, route);
    let outcome = run(&fixture);
    let payload = payload(&outcome);

    assert_eq!(text(field(&payload, "model")), model);
    assert_eq!(text(entry(&outcome, "model")), model);
}

#[test]
fn cursor_treats_only_a_fully_braced_part_as_a_wildcard() {
    assert!(path_matches(
        "/v0/agents/{agent_id}/stop",
        "/v0/agents/bc-1/stop"
    ));
    assert!(!path_matches("/v0/agents/{id/stop", "/v0/agents/bc-1/stop"));
    assert!(!path_matches("/v0/agents/id}/stop", "/v0/agents/bc-1/stop"));
}

#[rstest]
#[case::all_fields(
    r#"{"id": "bc-1", "name": "Fixer", "status": "active"}"#,
    "id=bc-1, name=Fixer, status=active"
)]
#[case::name_only(r#"{"name": "Fixer"}"#, "name=Fixer")]
#[case::status_only(r#"{"status": "active"}"#, "status=active")]
#[case::empty_strings(r#"{"id": "", "name": "", "status": ""}"#, RESULT)]
#[case::empty_object("{}", RESULT)]
#[case::null_fields(r#"{"id": null, "name": null}"#, RESULT)]
#[case::non_string_fields(r#"{"id": 7, "name": ["Fixer"], "status": true}"#, RESULT)]
#[case::list_body("[]", RESULT)]
#[case::null_body("null", RESULT)]
fn cursor_summarizes_the_agent_fields_or_the_relayed_result(
    #[case] body: &str,
    #[case] expected: &str,
) {
    let fixture = Fixture::new("GET", "/v0/agents/bc-1").body(json(body));
    let outcome = run(&fixture);

    assert_eq!(text(field(&result_json(&outcome), "response")), expected);
    assert_eq!(
        text(field(field(&payload(&outcome), "response"), "response")),
        expected
    );
}

#[test]
fn cursor_logs_zero_cost_and_leaves_the_provider_out_of_the_kwargs() {
    let fixture = Fixture::new("POST", "/v0/agents").body(json(r#"{"id": "bc-1"}"#));
    let outcome = run(&fixture);
    let payload = payload(&outcome);
    let call_details = details(&fixture.logging_obj);

    assert_eq!(text(field(&payload, "model")), "cursor/cursor:agent:create");
    assert_eq!(decimal(field(&payload, "response_cost")), Decimal::ZERO);
    assert_eq!(decimal(entry(&outcome, "response_cost")), Decimal::ZERO);
    assert_eq!(decimal(&call_details["response_cost"]), Decimal::ZERO);
    assert_eq!(
        text(field(field(&payload, "response"), "response")),
        "id=bc-1"
    );
    assert!(!outcome.kwargs.entries.contains_key("custom_llm_provider"));
    assert!(matches!(field(&payload, "custom_llm_provider"), Json::Null));
}

#[test]
fn cursor_writes_the_model_and_provider_into_the_call_details() {
    let fixture = Fixture::new("GET", "/v0/models");
    let outcome = run(&fixture);
    let payload = payload(&outcome);
    let call_details = details(&fixture.logging_obj);

    assert_eq!(
        logged_model(&fixture.logging_obj),
        "cursor/cursor:models:list"
    );
    assert_eq!(text(&call_details["model"]), "cursor/cursor:models:list");
    assert_eq!(text(&call_details["custom_llm_provider"]), "cursor");
    assert_eq!(text(field(&payload, "model")), "cursor/cursor:models:list");
    assert_eq!(text(field(&payload, "call_type")), "pass_through_endpoint");
}

#[test]
fn cursor_keeps_the_callers_kwargs() {
    let mut caller = IndexMap::new();
    caller.insert(
        "litellm_call_id".to_owned(),
        Json::Text("call-9".to_owned()),
    );
    let fixture =
        Fixture::new("GET", "/v0/me").caller_kwargs(PassThroughLoggingKwargs { entries: caller });
    let outcome = run(&fixture);

    assert_eq!(text(entry(&outcome, "litellm_call_id")), "call-9");
}

#[test]
fn cursor_logs_the_operation_and_agent_id_at_debug() {
    let (logs, _guard) = capture_logs(tracing::Level::DEBUG);
    let fixture = Fixture::new("GET", "/v0/agents/bc-1").body(json(r#"{"id": "bc-1"}"#));
    let outcome = run(&fixture);

    assert_eq!(
        text(field(&payload(&outcome), "model")),
        "cursor/cursor:agent:status"
    );
    let contents = logs.contents();
    assert!(contents.contains("operation=cursor:agent:status"));
    assert!(contents.contains("agent_id=bc-1"));
}

#[test]
fn cursor_warns_and_returns_the_raw_result_when_the_route_is_not_a_url() {
    let (logs, _guard) = capture_logs(tracing::Level::ERROR);
    let mut entries = IndexMap::new();
    entries.insert("model".to_owned(), Json::Text("caller-model".to_owned()));
    let caller = PassThroughLoggingKwargs { entries };
    let fixture =
        Fixture::new("GET", "https://api.cursor.com:bad/v0/me").caller_kwargs(caller.clone());
    let outcome = run(&fixture);

    let contents = logs.contents();
    assert!(contents.contains("Error in Cursor passthrough logging handler"));
    assert!(contents.contains("Invalid port"));
    assert_eq!(outcome.kwargs, caller);
    assert_eq!(text(field(&result_json(&outcome), "response")), RESULT);
    let call_details = details(&fixture.logging_obj);
    assert!(logged_model(&fixture.logging_obj).is_empty());
    assert!(text(&call_details["model"]).is_empty());
    assert!(!call_details.contains_key("response_cost"));
}
