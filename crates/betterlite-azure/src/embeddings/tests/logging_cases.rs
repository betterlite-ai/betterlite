use std::sync::Mutex;
use std::time::Duration;

use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
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

use crate::embeddings::client::Fields;
use crate::embeddings::logging::{
    EmbeddingLogger as _, PostCallRecord, PreCallRecord, ResponseKind,
};

#[derive(Default)]
struct Sink(Mutex<Vec<LogRecord>>);

impl LogSink for Sink {
    fn emit(&self, record: LogRecord) {
        self.0.lock().unwrap().push(record);
    }
}

fn state(redactor: &Redactor) -> LoggingState {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"my-deployment","call_type":"embedding","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .unwrap();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("generated"),
        Duration::default,
    )
    .unwrap()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn body() -> Fields {
    Fields::from([("model".to_owned(), text("my-deployment"))])
}

fn arguments(state: &LoggingState) -> &IndexMap<String, Json> {
    let Some(Json::Object(arguments)) = state.model_call_details.get("additional_args") else {
        panic!("the call records additional_args");
    };
    arguments
}

fn environment<'env>(redactor: &'env Redactor, sink: &'env Sink) -> PrePostEnv<'env> {
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
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").unwrap(),
    }
}

fn pre_call_records_the_request(recorded: &mut LoggingState, env: &PrePostEnv<'_>) {
    LogContext {
        state: recorded,
        env,
    }
    .pre_call(PreCallRecord {
        input: Json::List(vec![text("a")]),
        api_key: Some(SecretString::from("azure-key-value")),
        azure_ad_token: None,
        complete_input_dict: body(),
    })
    .unwrap();
    assert_eq!(
        recorded.model_call_details.get("log_event_type"),
        Some(&text("pre_api_call"))
    );
    assert_eq!(
        arguments(recorded).get("complete_input_dict"),
        Some(&Json::Object(body()))
    );
    let Some(Json::Object(headers)) = arguments(recorded).get("headers") else {
        panic!("the pre call records the headers");
    };
    assert_eq!(headers.get("azure_ad_token"), Some(&Json::Null));
    assert_ne!(headers.get("api_key"), Some(&text("azure-key-value")));
}

fn post_call_records_the_response(recorded: &mut LoggingState, env: &PrePostEnv<'_>) {
    LogContext {
        state: recorded,
        env,
    }
    .post_call(PostCallRecord {
        input: Json::List(vec![text("a")]),
        api_key: None,
        original_response: Json::Object(body()),
        kind: ResponseKind::Dumped,
        complete_input_dict: body(),
        api_base: Some("https://example.openai.azure.com".to_owned()),
    });
    assert_eq!(
        recorded.model_call_details.get("log_event_type"),
        Some(&text("post_api_call"))
    );
    let Some(Json::Text(response)) = recorded.model_call_details.get("original_response") else {
        panic!("the post call records the response text");
    };
    assert!(response.contains("my-deployment"));
    assert_eq!(
        arguments(recorded).get("api_base"),
        Some(&text("https://example.openai.azure.com"))
    );
}

pub(super) fn logs_pre_and_post_through_the_logging_crate() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut recorded = state(&redactor);
    let sink = Sink::default();
    let env = environment(&redactor, &sink);
    pre_call_records_the_request(&mut recorded, &env);
    post_call_records_the_response(&mut recorded, &env);
}
