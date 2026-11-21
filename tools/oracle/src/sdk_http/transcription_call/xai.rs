use std::time::Duration;

use betterlite_context::non_default_params::Params;
use betterlite_core::api::transcription_call::dispatch::{
    TranscriptionAnswer, TranscriptionProviderReply,
};
use betterlite_core::api::transcription_call::target::InjectedTranscription;
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_core::api::vector_store_http_write::service::LogContext;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::{Headers as CallerHeaders, Object};
use betterlite_provider::api::transcription_contract::AudioFile;
use betterlite_provider::api::transcription_http::config::{ClientRequest, ClientSource, Headers};
use betterlite_provider::api::transcription_http::error::TranscriptionHttpError;
use betterlite_provider::api::transcription_http::reply::TranscriptionReply;
use betterlite_provider::api::transcription_http::{
    TranscriptionCall as HttpCall, TranscriptionDeps, audio_transcriptions,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use secrecy::ExposeSecret;

use self::payload::XaiConfig;
use super::super::error::SdkHttpError;
use super::DispatchFailure;

mod payload;

// B2453's generic handler over xAI's own transforms, the operation a native config selection reaches
pub(super) async fn transcribe(
    operation: InjectedTranscription<'_>,
    client: &ProxiedClient,
) -> Result<TranscriptionAnswer, DispatchFailure> {
    let input = operation.input;
    let optional_params = object(&input.optional_params);
    let litellm_params = object(&input.litellm_params);
    let headers = request_headers(operation.headers);
    let api_key = input.api_key.map(ExposeSecret::expose_secret);
    let call = HttpCall {
        model: input.model,
        audio_file: AudioFile {
            filename: input.file.filename.as_str(),
            content: input.file.file_content.as_slice(),
            content_type: input.file.content_type.as_str(),
        },
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        api_key,
        api_base: input.api_base,
        custom_llm_provider: "xai",
        headers: Some(&headers),
        atranscription: Json::Bool(input.atranscription),
        timeout: Some(input.timeout),
        max_retries: input.max_retries,
        shared_session: input.shared_session,
    };
    let config = XaiConfig;
    let source = Source {
        client: client.clone(),
    };
    let deps = TranscriptionDeps {
        config: Some(&config),
        source: &source,
        supplied: None,
    };
    let mut fixture = Fixture::new().map_err(|error| DispatchFailure::Unsupported {
        message: error.to_string(),
    })?;
    answer(fixture.run(&deps, &call).await)
}

fn answer(
    result: Result<TranscriptionReply, TranscriptionHttpError>,
) -> Result<TranscriptionAnswer, DispatchFailure> {
    match result {
        Ok(reply) => Ok(TranscriptionAnswer::Reply(Box::new(
            TranscriptionProviderReply {
                response: reply.response,
                provider_headers: reply.provider_headers,
            },
        ))),
        Err(TranscriptionHttpError::Provider(error)) => Err(DispatchFailure::Provider {
            status: Some(error.status_code),
            message: error.message,
        }),
        Err(other) => Err(DispatchFailure::Unsupported {
            message: other.to_string(),
        }),
    }
}

// The core hands Params; the generic handler reads objects the way json.loads left them
fn object(params: &Params) -> Object {
    let mut object = Object::new();
    for (name, value) in params {
        object.insert(
            name.clone(),
            serde_json::to_value(value).unwrap_or_default(),
        );
    }
    object
}

fn request_headers(caller: Option<&CallerHeaders>) -> Headers {
    let mut headers = Headers::new();
    for (name, value) in caller.into_iter().flatten() {
        headers.insert(name.clone(), value.clone());
    }
    headers
}

struct Source {
    client: ProxiedClient,
}

impl ClientSource for Source {
    type Client = ProxiedClient;

    fn client(&self, _request: ClientRequest<'_>) -> Result<Self::Client, ClientError> {
        Ok(self.client.clone())
    }
}

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// The logging state a production native request would carry, with nothing attached to it
struct Fixture {
    redactor: Redactor,
    sink: Sink,
    state: LoggingState,
    now: DateTime<FixedOffset>,
}

impl Fixture {
    fn new() -> Result<Self, SdkHttpError> {
        let init: LoggingInit = serde_json::from_str(
            r#"{"model":"xai/whisper-1","call_type":"transcription","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
        )
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
        let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
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
        .map_err(SdkHttpError::LoggingState)?;
        Ok(Self {
            redactor,
            sink: Sink,
            state,
            now,
        })
    }

    async fn run(
        &mut self,
        deps: &TranscriptionDeps<'_, XaiConfig, Source>,
        call: &HttpCall<'_>,
    ) -> Result<TranscriptionReply, TranscriptionHttpError> {
        let env = PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: &self.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: self.now,
        };
        let mut log = LogContext {
            state: &mut self.state,
            env: &env,
        };
        audio_transcriptions(deps, call, &mut log).await
    }
}
