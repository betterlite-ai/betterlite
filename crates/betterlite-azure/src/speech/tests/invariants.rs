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

use crate::speech::binary::{AudioSpeechCall, finish_audio_speech};
use crate::speech::dispatch::SpeechGlobals;
use crate::speech::logging::{SpeechLogger as _, SpeechPreCall};

use super::support::{self, API_BASE, Inputs, Recorder, SECRET, json};

// Two credentials, or one credential in its sync and async form, never share a client entry or its headers.
#[test]
fn azure_speech_credential_identity_isolation() {
    let mut inputs = Inputs::default();
    let first = credentialed("sk-first-key", API_BASE);
    let first_request = support::start(&mut inputs, &first, &mut Recorder::default())
        .expect("the first request builds");
    let second = credentialed("sk-second-key", API_BASE);
    let second_request = support::start(&mut inputs, &second, &mut Recorder::default())
        .expect("the second request builds");
    assert_eq!(
        support::header(&first_request, "api-key"),
        Some("sk-first-key")
    );
    assert_eq!(
        support::header(&second_request, "api-key"),
        Some("sk-second-key")
    );
    assert_eq!(inputs.cache.entries.len(), 2);
    assert_ne!(inputs.cache.entries[0].0, inputs.cache.entries[1].0);
    let _repeat = support::start(&mut inputs, &first, &mut Recorder::default())
        .expect("the repeat request builds");
    assert_eq!(inputs.cache.entries.len(), 2);
    let moved = credentialed("sk-first-key", "https://other.openai.azure.com");
    let moved_request = support::start(&mut inputs, &moved, &mut Recorder::default())
        .expect("the moved request builds");
    assert_eq!(moved_request.url.host_str(), Some("other.openai.azure.com"));
    assert_eq!(
        support::header(&moved_request, "api-key"),
        Some("sk-first-key")
    );
    assert_eq!(inputs.cache.entries.len(), 2);
    let mut asynchronous = credentialed("sk-first-key", API_BASE);
    asynchronous.aspeech = true;
    let asynchronous_request = support::start(&mut inputs, &asynchronous, &mut Recorder::default())
        .expect("the async request builds");
    assert_eq!(
        support::header(&asynchronous_request, "api-key"),
        Some("sk-first-key")
    );
    assert_eq!(inputs.cache.entries.len(), 3);
}

#[test]
fn azure_speech_secret_safe_failure() {
    let globals = SpeechGlobals {
        api_base: Some(String::from("https://x.example")),
        api_key: Some(SecretString::from(SECRET)),
        azure_key: Some(SecretString::from(SECRET)),
    };
    assert!(!format!("{globals:?}").contains(SECRET));
    let mut call = support::call(
        "tts",
        "hello",
        "alloy",
        support::pairs(&[("extra_headers", json(r#"{"x-trace":"bad\nvalue"}"#))]),
    );
    call.api_key = Some(SecretString::from(SECRET));
    let error = support::start(&mut Inputs::default(), &call, &mut Recorder::default())
        .expect_err("an invalid header value fails the request");
    let (class, _status) = error.kind();
    assert_eq!(class, "Exception");
    assert!(!format!("{error:?} {error}").contains(SECRET));
    let Err(failed) = finish_audio_speech(support::response(401, &[], b"denied")) else {
        panic!("the status fails the call");
    };
    let (failed_class, _failed_status) = failed.kind();
    assert_eq!(failed_class, "APIStatusError");
    let shown = format!("{failed:?} {failed}");
    assert!(!shown.contains(SECRET));
    assert!(shown.contains("denied"));
}

fn credentialed(key: &'static str, api_base: &'static str) -> AudioSpeechCall<'static> {
    let mut call = support::call("tts", "hello", "alloy", IndexMap::new());
    call.api_key = Some(SecretString::from(key));
    call.api_base = Some(String::from(api_base));
    call
}

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

fn logging_init() -> LoggingInit {
    serde_json::from_str(
        r#"{"model":"tts-1","call_type":"speech","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .expect("the logging fixture is valid")
}

fn record_speech_pre_call(state: &mut LoggingState, env: &PrePostEnv<'_>) {
    let mut logger = LogContext { state, env };
    let record = SpeechPreCall {
        input: "hello",
        api_base: API_BASE,
        data: IndexMap::from([(String::from("voice"), json(r#""alloy""#))]),
    };
    logger.pre_call(record).expect("pre_call records");
}

fn assert_pre_call_details(state: &LoggingState) {
    let details = &state.model_call_details;
    assert_eq!(
        details.get("log_event_type"),
        Some(&json(r#""pre_api_call""#))
    );
    assert_eq!(details.get("input"), Some(&json(r#""hello""#)));
    let Some(Json::Object(args)) = details.get("additional_args") else {
        panic!("pre_call records additional_args");
    };
    assert_eq!(
        args.get("api_base"),
        Some(&json(r#""https://example.openai.azure.com""#))
    );
    assert_eq!(
        args.get("complete_input_dict"),
        Some(&json(r#"{"voice":"alloy"}"#))
    );
    assert_eq!(
        state.litellm_params.get("api_base"),
        Some(&json(r#""https://example.openai.azure.com""#))
    );
}

// LogContext hands the speech pre-call to B249's pre_call, which fills the call details from it
#[test]
fn speech_pre_post_logger_records_the_pre_call() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
    let mut state = LoggingState::new(
        logging_init(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated"),
        Duration::default,
    )
    .expect("the logging state builds");
    let sink = Sink::default();
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &redactor,
        log: &sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("the clock parses"),
    };
    record_speech_pre_call(&mut state, &env);
    assert_pre_call_details(&state);
}
