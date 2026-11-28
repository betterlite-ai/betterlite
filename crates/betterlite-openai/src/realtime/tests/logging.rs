use std::sync::Mutex;
use std::time::Duration;

use betterlite_config::tls::Verification;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::realtime::connect::{RealtimeConnectSettings, prepare_connection};
use crate::realtime::error::RealtimeError;
use crate::realtime::logging::{RealtimeLogRecord, RealtimeLogger};
use crate::realtime::url::QueryParams;

#[derive(Default)]
struct Sink(Mutex<Vec<LogRecord>>);

impl LogSink for Sink {
    fn emit(&self, record: LogRecord) {
        self.0
            .lock()
            .expect("the sink lock is not poisoned")
            .push(record);
    }
}

fn state() -> (Redactor, LoggingState) {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"realtime-a","call_type":"realtime","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .expect("the logging fixture is valid");
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
    let state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated"),
        Duration::default,
    )
    .expect("the logging state builds");
    (redactor, state)
}

fn env<'setup>(redactor: &'setup Redactor, sink: &'setup Sink) -> PrePostEnv<'setup> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("the clock parses"),
    }
}

#[test]
fn records_the_realtime_pre_call() {
    let (redactor, mut state) = state();
    let sink = Sink::default();
    let env = env(&redactor, &sink);
    let key = SecretString::from("sk-test".to_owned());
    let settings = RealtimeConnectSettings {
        api_base: None,
        api_key: Some(&key),
        model: "realtime-a",
        query_params: None,
        beta_realtime: true,
        ssl_verification: &Verification::Enabled,
        max_message_size_bytes: 4_096,
    };
    let mut logger = LogContext {
        state: &mut state,
        env: &env,
    };
    prepare_connection(&settings, &mut logger).expect("the connection prepares");
    assert_pre_call_details(&state);
}

fn assert_pre_call_details(state: &LoggingState) {
    assert_eq!(
        state.model_call_details.get("log_event_type"),
        Some(&Json::Text(String::from("pre_api_call")))
    );
    assert_eq!(state.model_call_details.get("input"), Some(&Json::Null));
    let Some(Json::Object(arguments)) = state.model_call_details.get("additional_args") else {
        panic!("the pre call records additional_args");
    };
    assert_eq!(
        arguments.get("api_base"),
        Some(&Json::Text(String::from(
            "wss://api.openai.com/v1/realtime?model=realtime-a"
        )))
    );
    let Some(Json::Object(recorded_headers)) = arguments.get("headers") else {
        panic!("the pre call records headers");
    };
    assert_eq!(
        recorded_headers.get("OpenAI-Beta"),
        Some(&Json::Text(String::from("realtime=v1")))
    );
    let Some(Json::Text(authorization)) = recorded_headers.get("Authorization") else {
        panic!("the pre call records the Authorization header");
    };
    assert_ne!(authorization.as_str(), "Bearer sk-test");
    let Some(Json::Object(body)) = arguments.get("complete_input_dict") else {
        panic!("the pre call records complete_input_dict");
    };
    assert_eq!(
        body.get("query_params"),
        Some(&Json::Object(IndexMap::from([(
            String::from("model"),
            Json::Text(String::from("realtime-a")),
        )])))
    );
}

struct FailingLogger;

impl RealtimeLogger for FailingLogger {
    fn pre_call(&mut self, _record: RealtimeLogRecord<'_>) -> Result<(), PrePostError> {
        Err(PrePostError::RawRequestCaptured)
    }
}

#[test]
fn pre_call_failure_stops_the_connection() {
    let key = SecretString::from("sk-test".to_owned());
    let settings = RealtimeConnectSettings {
        api_base: None,
        api_key: Some(&key),
        model: "realtime-a",
        query_params: None,
        beta_realtime: true,
        ssl_verification: &Verification::Enabled,
        max_message_size_bytes: 4_096,
    };
    assert!(matches!(
        prepare_connection(&settings, &mut FailingLogger),
        Err(RealtimeError::Logging {
            source: PrePostError::RawRequestCaptured
        })
    ));
}

#[test]
fn records_a_null_query_value_as_null() {
    let (redactor, mut state) = state();
    let sink = Sink::default();
    let env = env(&redactor, &sink);
    let key = SecretString::from("sk-test".to_owned());
    let query: QueryParams = IndexMap::from([(String::from("intent"), None)]);
    let settings = RealtimeConnectSettings {
        api_base: None,
        api_key: Some(&key),
        model: "realtime-a",
        query_params: Some(&query),
        beta_realtime: false,
        ssl_verification: &Verification::Enabled,
        max_message_size_bytes: 4_096,
    };
    let mut logger = LogContext {
        state: &mut state,
        env: &env,
    };
    prepare_connection(&settings, &mut logger).expect("the connection prepares");
    let Some(Json::Object(arguments)) = state.model_call_details.get("additional_args") else {
        panic!("the pre call records additional_args");
    };
    let Some(Json::Object(body)) = arguments.get("complete_input_dict") else {
        panic!("the pre call records complete_input_dict");
    };
    assert_eq!(
        body.get("query_params"),
        Some(&Json::Object(IndexMap::from([(
            String::from("intent"),
            Json::Null,
        )])))
    );
}
