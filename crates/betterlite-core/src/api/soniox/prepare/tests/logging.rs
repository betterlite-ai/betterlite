use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

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
use betterlite_openai::api::audio_file_input::AudioFile;
use betterlite_openai::openai_base_config::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use rstest::rstest;

use super::super::logging::{
    REDACTED_VALUE, SECRET_FIELD, SonioxEvent, SonioxLogger, SonioxPostCall, SonioxPreCall,
    log_post_call, log_pre_call, redacted_body,
};

#[derive(Default)]
struct Recorder {
    failing: bool,
    pre: Option<SonioxPreCall>,
    post: Option<SonioxPostCall>,
}

impl Recorder {
    fn fail(&self) -> Result<(), PrePostError> {
        if self.failing {
            return Err(PrePostError::CallbackFailed {
                message: String::from("callback boom"),
            });
        }
        Ok(())
    }
}

impl SonioxLogger for Recorder {
    fn record(&mut self, event: SonioxEvent) -> Result<(), PrePostError> {
        self.fail()?;
        match event {
            SonioxEvent::PreCall(record) => self.pre = Some(record),
            SonioxEvent::PostCall(record) => self.post = Some(record),
        }
        Ok(())
    }
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

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the body is a JSON object")
}

fn state() -> (Redactor, LoggingState) {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"stt-async-v4","call_type":"atranscription","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
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

fn post_call_input(audio: Option<&AudioFile<'_>>) -> Json {
    let body = object(r#"{"model": "stt-async-v4"}"#);
    let response = object(r#"{"transcription": {}}"#);
    let mut recorder = Recorder {
        failing: false,
        pre: None,
        post: None,
    };

    log_post_call(&mut recorder, audio, &response, &body);

    recorder.post.expect("the post call is recorded").input
}

#[rstest]
#[case::text(r#""super-secret""#)]
#[case::true_flag("true")]
#[case::false_flag("false")]
#[case::zero("0")]
#[case::empty_text(r#""""#)]
#[case::list(r#"["a"]"#)]
#[case::object(r#"{"k":"v"}"#)]
fn redacts_every_non_null_secret_value(#[case] value: &str) {
    let body = object(&format!(
        r#"{{"model": "m", "webhook_auth_header_value": {value}}}"#
    ));

    let redacted = redacted_body(&body);

    assert_eq!(
        redacted.get(SECRET_FIELD),
        Some(&Json::Text(String::from(REDACTED_VALUE)))
    );
    assert_eq!(redacted.get("model"), Some(&Json::Text(String::from("m"))));
    assert_eq!(
        body.get(SECRET_FIELD).map(ToString::to_string),
        Some(String::from(value))
    );
}

#[test]
fn keeps_a_null_secret_null() {
    let body = object(r#"{"model": "m", "webhook_auth_header_value": null}"#);

    let redacted = redacted_body(&body);

    assert_eq!(redacted.get(SECRET_FIELD), Some(&Json::Null));
}

#[test]
fn adds_no_secret_field() {
    let body = object(r#"{"model": "m"}"#);

    let redacted = redacted_body(&body);

    assert_eq!(redacted.len(), 1);
    assert!(!redacted.contains_key(SECRET_FIELD));
}

#[test]
fn leaves_a_nested_secret_untouched() {
    let body = object(r#"{"context": {"webhook_auth_header_value": "secret"}}"#);

    let redacted = redacted_body(&body);

    let Some(Json::Object(context)) = redacted.get("context") else {
        panic!("the context is an object");
    };
    assert_eq!(
        context.get(SECRET_FIELD),
        Some(&Json::Text(String::from("secret")))
    );
}

#[test]
fn logs_the_pre_call_with_the_redacted_body() {
    let body = object(r#"{"model": "stt-async-v4", "webhook_auth_header_value": "secret"}"#);
    let mut recorder = Recorder {
        failing: false,
        pre: None,
        post: None,
    };

    log_pre_call(&mut recorder, "https://api.soniox.test", &body);

    let record = recorder.pre.expect("the pre call is recorded");
    assert_eq!(record.url, "https://api.soniox.test/v1/transcriptions");
    assert_eq!(
        record.body.get(SECRET_FIELD),
        Some(&Json::Text(String::from(REDACTED_VALUE)))
    );
    assert_eq!(
        record.body.get("model"),
        Some(&Json::Text(String::from("stt-async-v4")))
    );
}

#[test]
fn suppresses_a_failing_pre_call() {
    let body = object(r#"{"model": "stt-async-v4"}"#);
    let mut recorder = Recorder {
        failing: true,
        pre: None,
        post: None,
    };

    log_pre_call(&mut recorder, "https://api.soniox.test", &body);

    assert!(recorder.pre.is_none());
}

#[test]
fn logs_the_post_call_with_the_audio_name() {
    let body = object(r#"{"model": "stt-async-v4", "webhook_auth_header_value": "secret"}"#);
    let response = object(r#"{"transcription": {"status": "completed"}}"#);
    let mut recorder = Recorder {
        failing: false,
        pre: None,
        post: None,
    };

    log_post_call(
        &mut recorder,
        Some(&AudioFile::Text("clip.wav")),
        &response,
        &body,
    );

    let record = recorder.post.expect("the post call is recorded");
    assert_eq!(record.input, Json::Text(String::from("clip.wav")));
    assert_eq!(
        record.body.get(SECRET_FIELD),
        Some(&Json::Text(String::from(REDACTED_VALUE)))
    );
    assert!(matches!(
        &record.original_response,
        Json::Object(fields) if fields.contains_key("transcription")
    ));
}

#[test]
fn logs_no_input_for_a_missing_audio_file() {
    assert_eq!(post_call_input(None), Json::Null);
}

#[test]
fn logs_no_input_for_an_empty_text() {
    assert_eq!(post_call_input(Some(&AudioFile::Text(""))), Json::Null);
}

#[test]
fn logs_no_input_for_empty_bytes() {
    assert_eq!(
        post_call_input(Some(&AudioFile::ByteArray(b""))),
        Json::Null
    );
}

#[test]
fn logs_the_file_name_of_bytes() {
    assert_eq!(
        post_call_input(Some(&AudioFile::Bytes(b"abc"))),
        Json::Text(String::from("audio.wav"))
    );
}

#[test]
fn logs_the_file_name_of_a_path() {
    let path = Path::new("/audio/clip.wav");

    assert_eq!(
        post_call_input(Some(&AudioFile::Path(path))),
        Json::Text(String::from("/audio/clip.wav"))
    );
}

#[test]
fn suppresses_a_failing_post_call() {
    let body = object(r#"{"model": "stt-async-v4"}"#);
    let response = object(r#"{"transcription": {}}"#);
    let mut recorder = Recorder {
        failing: true,
        pre: None,
        post: None,
    };

    log_post_call(&mut recorder, None, &response, &body);

    assert!(recorder.post.is_none());
}

#[test]
fn records_the_pre_call_through_b249() {
    let (redactor, mut state) = state();
    let sink = Sink::default();
    let env = env(&redactor, &sink);
    let body = object(r#"{"model": "stt-async-v4", "webhook_auth_header_value": "secret"}"#);

    {
        let mut logger = LogContext {
            state: &mut state,
            env: &env,
        };
        log_pre_call(&mut logger, "https://api.soniox.test", &body);
    };

    let details = &state.model_call_details;
    let Some(Json::Object(additional)) = details.get("additional_args") else {
        panic!("additional_args is an object");
    };
    assert_eq!(
        additional.get("api_base"),
        Some(&Json::Text(String::from(
            "https://api.soniox.test/v1/transcriptions"
        )))
    );
    assert_eq!(additional.get("atranscription"), Some(&Json::Bool(true)));
    let Some(Json::Object(recorded)) = additional.get("complete_input_dict") else {
        panic!("the complete input dict is an object");
    };
    assert_eq!(
        recorded.get(SECRET_FIELD),
        Some(&Json::Text(String::from(REDACTED_VALUE)))
    );
    assert_eq!(details.get("input"), Some(&Json::Null));
    assert!(!details.contains_key("api_key"));
}

#[test]
fn records_the_post_call_through_b249() {
    let (redactor, mut state) = state();
    let sink = Sink::default();
    let env = env(&redactor, &sink);
    let body = object(r#"{"model": "stt-async-v4", "webhook_auth_header_value": "secret"}"#);
    let response = object(r#"{"transcription": {"status": "completed"}}"#);

    {
        let mut logger = LogContext {
            state: &mut state,
            env: &env,
        };
        log_post_call(
            &mut logger,
            Some(&AudioFile::Text("clip.wav")),
            &response,
            &body,
        );
    };

    let expected_json: Json = serde_json::from_str(
        r#"{
            "litellm_trace_id": "trace-1", "litellm_call_id": "call-1",
            "input": "clip.wav", "litellm_params": {}, "applied_guardrails": null,
            "model": "stt-async-v4", "log_event_type": "post_api_call",
            "original_response": "{\"transcription\": {\"status\": \"completed\"}}",
            "additional_args": {"complete_input_dict":
                {"model": "stt-async-v4", "webhook_auth_header_value": "[REDACTED]"}}
        }"#,
    )
    .expect("the expected details are JSON");
    let Json::Object(expected) = expected_json else {
        panic!("the expected details are an object");
    };

    assert_eq!(state.model_call_details, expected);
}
