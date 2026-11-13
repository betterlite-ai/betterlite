use std::sync::Mutex;
use std::time::Duration;

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
use rstest::rstest;
use uuid::Uuid;

use super::fixture::{AudioFixture, ConnectionOptions, Fixture, Reply, connection};
use crate::api::audio_file_input::{ProcessedAudioFile, get_audio_file_name};
use crate::transcription::call::{
    TranscriptionCall, TranscriptionRequest, async_audio_transcriptions, audio_transcriptions,
    finish_transcription,
};
use crate::transcription::config::TranscriptionFamily;
use crate::transcription::logging::PrePostLogger;
use crate::transcription::request::prepare_data;
use crate::transcription::transport::TranscriptionTransport as _;

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
        r#"{"model":"whisper-1","call_type":"atranscription","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
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

fn request() -> TranscriptionRequest<'static> {
    TranscriptionRequest {
        model: "whisper-1",
        family: Some(TranscriptionFamily::Whisper),
        optional_params: IndexMap::from([(
            String::from("language"),
            Json::Text(String::from("en")),
        )]),
        atranscription: false,
    }
}

fn call<'file>(audio: &AudioFixture, file: &'file ProcessedAudioFile) -> TranscriptionCall<'file> {
    TranscriptionCall {
        input: get_audio_file_name(&audio.audio_file()),
        file,
        connection: connection(&ConnectionOptions {
            api_key: Some("sk-transcription"),
            environment_api_key: None,
            shared_session: false,
        }),
        boundary: Uuid::from_u128(11),
    }
}

fn pre_call_additional_args(is_async: bool) -> Json {
    let audio = AudioFixture::new("a.wav", b"abc", "audio/wav");
    let processed = audio.processed();
    let (redactor, mut state) = state();
    let sink = Sink::default();
    let env = env(&redactor, &sink);
    let mut logger = PrePostLogger {
        state: &mut state,
        env: &env,
    };
    let started = if is_async {
        let data = prepare_data(
            Some(TranscriptionFamily::Whisper),
            "whisper-1",
            &IndexMap::new(),
        )
        .expect("the data builds");
        async_audio_transcriptions(&call(&audio, &processed), &data, &mut logger)
    } else {
        audio_transcriptions(&call(&audio, &processed), &request(), &mut logger)
    };
    started.expect("the call begins");
    state
        .model_call_details
        .get("additional_args")
        .expect("the pre call recorded additional_args")
        .clone()
}

// handler.py logs atranscription=True from both the sync and the async call site
#[rstest]
#[case::sync_call(false)]
#[case::async_call(true)]
fn records_the_transcription_pre_call(#[case] is_async: bool) {
    let Json::Object(fields) = pre_call_additional_args(is_async) else {
        panic!("additional_args is an object");
    };
    assert_eq!(fields.get("atranscription"), Some(&Json::Bool(true)));
    assert!(fields.contains_key("complete_input_dict"));
}

#[tokio::test]
async fn records_the_transcription_post_call() {
    let audio = AudioFixture::new("a.wav", b"abc", "audio/wav");
    let processed = audio.processed();
    let (redactor, mut state) = state();
    let sink = Sink::default();
    let env = env(&redactor, &sink);
    let fixture = Fixture::new(Reply::json(r#"{"text":"hi","language":"en"}"#));
    let mut logger = PrePostLogger {
        state: &mut state,
        env: &env,
    };
    let (outbound, pending) =
        audio_transcriptions(&call(&audio, &processed), &request(), &mut logger)
            .expect("the call begins");
    finish_transcription(pending, fixture.send(outbound).await, &mut logger)
        .expect("the reply finishes");
    let details = &state.model_call_details;
    assert_eq!(
        details.get("log_event_type"),
        Some(&Json::Text(String::from("post_api_call")))
    );
    assert_eq!(
        details.get("input"),
        Some(&Json::Text(String::from("(a.wav, bytes)")))
    );
    assert!(matches!(
        details.get("response_headers"),
        Some(Json::Object(fields)) if fields.contains_key("content-type")
    ));
    assert!(matches!(
        details.get("original_response"),
        Some(Json::Text(text)) if text.contains("\"text\": \"hi\"")
    ));
}
