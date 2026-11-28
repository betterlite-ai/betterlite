#![cfg(test)]

use std::sync::{Arc, Mutex};
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
use betterlite_openai::transcription::request::TranscriptionData;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::fixtures::{API_KEY, audio_file, text};
use crate::credentials::source::{CallableTokenProvider, ProviderFailure, TokenReply};
use crate::endpoints::projection::TokenProvider;
use crate::transcription::call::AzureTranscriptionError;
use crate::transcription::call::client::{RequestAuth, request_auth};
use crate::transcription::call::logging::{
    AzureTranscriptionLogger as _, AzureTranscriptionPostCall, AzureTranscriptionPreCall,
};
use crate::transcription::call::request::{OutboundTarget, outbound_request};

struct Scripted(TokenReply);

impl CallableTokenProvider for Scripted {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        Ok(self.0.clone())
    }
}

fn provider_auth(reply: TokenReply) -> Result<RequestAuth, AzureTranscriptionError> {
    let provider = TokenProvider(Arc::new(Scripted(reply)));
    request_auth(None, Some(&provider), Some(SecretString::from(API_KEY)))
}

// the SDK sends a provider's token as the bearer, and refuses an empty or non-string one before it reaches the api key
#[test]
fn token_provider_auth_needs_a_non_empty_text_token() {
    let Ok(RequestAuth::Bearer(token)) =
        provider_auth(TokenReply::Text(SecretString::from("ad-token")))
    else {
        panic!("a text token becomes the bearer");
    };
    assert_eq!(token.expose_secret(), "ad-token");
    let Err(empty) = provider_auth(TokenReply::Text(SecretString::from(""))) else {
        panic!("an empty token is refused");
    };
    assert_eq!(
        empty.to_string(),
        "Expected `azure_ad_token_provider` argument to return a string but it returned "
    );
    let Err(other) = provider_auth(TokenReply::NonText {
        type_name: String::from("int"),
    }) else {
        panic!("a non-string token is refused");
    };
    assert_eq!(
        other.to_string(),
        "Expected `azure_ad_token_provider` argument to return a string but it returned int"
    );
}

// a base that already names a deployment gets no second prefix, and its query survives with api-version replaced in place
#[test]
fn outbound_request_keeps_a_named_deployment_and_the_base_query() {
    let file = audio_file();
    let auth = RequestAuth::ApiKey(SecretString::from(API_KEY));
    let data = TranscriptionData {
        model: Some(text("whisper")),
        options: IndexMap::new(),
    };
    let request = outbound_request(
        &OutboundTarget {
            base_url: "https://example.openai.azure.com/openai/deployments/whisper/?a=1&&api-version=old&b=2",
            api_version: Some("2024-06-01"),
            auth: &auth,
            deadline: Duration::from_secs(30),
        },
        &data,
        &file,
        String::from("boundary"),
    )
    .expect("the request builds");
    assert_eq!(
        request.url.as_str(),
        "https://example.openai.azure.com/openai/deployments/whisper/audio/transcriptions?a=1&api-version=2024-06-01&b=2"
    );
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

fn logging_state(redactor: &Redactor) -> LoggingState {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"whisper","call_type":"atranscription","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .expect("the logging fixture is valid");
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
    .expect("the logging state builds")
}

fn logging_env<'setup>(redactor: &'setup Redactor, sink: &'setup Sink) -> PrePostEnv<'setup> {
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

// LogContext hands both records to B249's pre_call and post_call
#[test]
fn pre_post_logger_records_the_azure_calls() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
    let mut state = logging_state(&redactor);
    let sink = Sink::default();
    let env = logging_env(&redactor, &sink);
    let mut logger = LogContext {
        state: &mut state,
        env: &env,
    };
    logger
        .pre_call(AzureTranscriptionPreCall {
            input: "audio_file_1",
            authorization: "Bearer sk-fixture",
            api_base: "https://example.openai.azure.com/openai/",
            data: IndexMap::from([(String::from("model"), text("whisper"))]),
        })
        .expect("pre_call records");
    let pre_details = &logger.state.model_call_details;
    assert_eq!(
        pre_details.get("log_event_type"),
        Some(&text("pre_api_call"))
    );
    assert_eq!(pre_details.get("input"), Some(&text("audio_file_1")));
    let Some(Json::Object(pre_args)) = pre_details.get("additional_args") else {
        panic!("pre_call recorded additional_args");
    };
    assert_eq!(pre_args.get("atranscription"), Some(&Json::Bool(true)));
    assert!(pre_args.contains_key("complete_input_dict"));
    record_post_call(&mut logger);
}

fn record_post_call(logger: &mut LogContext<'_, '_>) {
    logger.post_call(AzureTranscriptionPostCall {
        input: "a.wav",
        authorization: None,
        api_base: None,
        original_response: text("hi"),
        data: None,
    });
    let post_details = &logger.state.model_call_details;
    assert_eq!(
        post_details.get("log_event_type"),
        Some(&text("post_api_call"))
    );
    assert_eq!(post_details.get("input"), Some(&text("a.wav")));
    assert_eq!(post_details.get("original_response"), Some(&text("hi")));
}
