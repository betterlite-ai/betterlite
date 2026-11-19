use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_config::types::{Config, ProxyTrust};
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::pool::ClientPool;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_openai::api::transcription_contract::{
    AudioTranscriptionData, AudioTranscriptionFileValue, AudioTranscriptionFiles,
    AudioTranscriptionRequestData, TranscriptionResponse,
};
use betterlite_openai::openai_base_config::Object;
use betterlite_provider::api::transcription_contract::{
    AudioFile, AudioTranscriptionConfig, TranscriptionContractError,
};
use betterlite_provider::api::transcription_http::config::{
    ClientRequest, ClientSource, ConfigFailure, Headers, TranscriptionHttpConfig, Validation,
};
use betterlite_provider::api::transcription_http::error::TranscriptionHttpError;
use betterlite_provider::api::transcription_http::reply::TranscriptionReply;
use betterlite_provider::api::transcription_http::{
    LogContext, TranscriptionCall, TranscriptionDeps, async_audio_transcriptions,
    audio_transcriptions,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};

use super::super::error::SdkHttpError;
use super::{Call, Mode, Transform};

// The pooled client a production request would get; the case env only carries the proxy settings
pub(super) struct Source {
    pool: ClientPool,
    tls: TlsPolicy,
    routing: Arc<ProxyPolicy>,
}

impl ClientSource for Source {
    type Client = ProxiedClient;

    fn client(&self, _request: ClientRequest<'_>) -> Result<ProxiedClient, ClientError> {
        self.pool
            .client(&self.tls, &self.routing)
            .map_err(ClientError::Pool)
    }
}

pub(super) fn source(
    config: &Config,
    env: &BTreeMap<String, String>,
) -> Result<Source, SdkHttpError> {
    let routing = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), env).map_err(SdkHttpError::ProxyPolicy)?,
    );
    let tls = TlsPolicy::resolve(config, env, None)
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    Ok(Source {
        pool: ClientPool::new(1, TransportLimits::DEFAULT),
        tls,
        routing,
    })
}

pub(super) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// The pre-call a production request runs: the same state a request would log through, with nothing attached to it
pub(super) struct Fixture {
    redactor: Redactor,
    sink: Sink,
    state: LoggingState,
    now: DateTime<FixedOffset>,
}

impl Fixture {
    pub(super) fn new() -> Result<Self, SdkHttpError> {
        let init: LoggingInit = serde_json::from_str(
            r#"{"model":"transcription","call_type":"transcription","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
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

    pub(super) async fn run<C, S>(
        &mut self,
        deps: &TranscriptionDeps<'_, C, S>,
        call: &TranscriptionCall<'_>,
        mode: Mode,
    ) -> Result<TranscriptionReply, TranscriptionHttpError>
    where
        C: TranscriptionHttpConfig,
        S: ClientSource,
    {
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
        match mode {
            Mode::Sync => audio_transcriptions(deps, call, &mut log).await,
            Mode::Asynchronous => async_audio_transcriptions(deps, call, &mut log).await,
        }
    }
}

// A provider config that answers with the case's request transform and response, as a real provider would
pub(super) struct FixtureConfig<'call> {
    call: &'call Call,
}

impl<'call> FixtureConfig<'call> {
    pub(super) const fn new(call: &'call Call) -> Self {
        Self { call }
    }
}

impl AudioTranscriptionConfig for FixtureConfig<'_> {
    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn transform_audio_request(
        &self,
        _model: &str,
        _audio_file: &AudioFile<'_>,
        _optional_params: &Object,
        _litellm_params: &Object,
    ) -> Result<AudioTranscriptionRequestData, TranscriptionContractError> {
        Ok(request_data(&self.call.transform))
    }

    fn supports_subtitle_synthesis(&self) -> bool {
        self.call.supports_subtitle_synthesis
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _model: &str,
        _optional_params: &Object,
        _litellm_params: &Object,
    ) -> String {
        format!("{}/audio/transcriptions", api_base.unwrap_or_default())
    }

    fn transform_audio_response(
        &self,
        _raw_response: &UpstreamResponse,
    ) -> Result<TranscriptionResponse, TranscriptionContractError> {
        Ok(fixture_response(self.call.response.as_ref()))
    }
}

impl TranscriptionHttpConfig for FixtureConfig<'_> {
    fn validate_environment(&self, input: &Validation<'_>) -> Result<Headers, ConfigFailure> {
        let mut headers = input.headers.clone();
        if let Some(key) = input.api_key {
            headers.insert(String::from("Authorization"), format!("Bearer {key}"));
        }
        Ok(headers)
    }
}

fn request_data(transform: &Transform) -> AudioTranscriptionRequestData {
    let files = transform.files.as_ref().map(|files| {
        files
            .iter()
            .map(|(name, text)| {
                (
                    name.clone(),
                    AudioTranscriptionFileValue::Text(text.clone()),
                )
            })
            .collect::<AudioTranscriptionFiles>()
    });
    let data = match &transform.raw {
        Some(raw) => AudioTranscriptionData::Bytes(raw.as_bytes().to_vec()),
        None => AudioTranscriptionData::Object(transform.data.clone().unwrap_or_default()),
    };
    AudioTranscriptionRequestData {
        data,
        files,
        content_type: transform.content_type.clone(),
    }
}

fn fixture_response(configured: Option<&TranscriptionResponse>) -> TranscriptionResponse {
    match configured {
        Some(fixture) => fixture.clone(),
        None => TranscriptionResponse {
            text: None,
            extra: Object::new(),
        },
    }
}
