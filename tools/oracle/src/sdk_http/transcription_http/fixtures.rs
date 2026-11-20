use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::{Config, ProxyTrust};
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_core::api::vector_store_http_write::service::LogContext;
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
use betterlite_openai::containers::container_file_http::request::SslVerify;
use betterlite_openai::openai_base_config::Object;
use betterlite_provider::api::transcription_contract::{
    AudioFile, AudioTranscriptionConfig, TranscriptionContractError,
};
use betterlite_provider::api::transcription_http::config::{
    ClientRequest, ClientSource, ConfigFailure, Headers, TranscriptionHttpConfig, UrlInput,
    Validation,
};
use betterlite_provider::api::transcription_http::error::TranscriptionHttpError;
use betterlite_provider::api::transcription_http::reply::TranscriptionReply;
use betterlite_provider::api::transcription_http::{
    TranscriptionCall, TranscriptionDeps, async_audio_transcriptions, audio_transcriptions,
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

    fn client(&self, request: ClientRequest<'_>) -> Result<ProxiedClient, ClientError> {
        // get_ssl_configuration: a false disables verification, a string names the CA bundle, true keeps the policy
        let tls = match request.ssl_verify.as_ref() {
            None | Some(SslVerify::Enabled(true)) => self.tls.clone(),
            Some(SslVerify::Enabled(false)) => TlsPolicy {
                verification: Verification::Disabled,
                ca_file: None,
                ..self.tls.clone()
            },
            Some(SslVerify::CaBundle(path)) => TlsPolicy {
                verification: Verification::CaFile(path.clone()),
                ca_file: Some(path.clone()),
                ..self.tls.clone()
            },
        };
        self.pool
            .client(&tls, &self.routing)
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
    pub(super) call: &'call Call,
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

    fn complete_transcription_url(&self, input: &UrlInput<'_>) -> Result<String, ConfigFailure> {
        let base = input.api_base.unwrap_or_default();
        Ok(format!("{base}/audio/transcriptions"))
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
    let data = transform.raw.as_ref().map_or_else(
        || AudioTranscriptionData::Object(transform.data.clone().unwrap_or_default()),
        |raw| AudioTranscriptionData::Bytes(raw.as_bytes().to_vec()),
    );
    AudioTranscriptionRequestData {
        data,
        files,
        content_type: transform.content_type.clone(),
    }
}

fn fixture_response(configured: Option<&TranscriptionResponse>) -> TranscriptionResponse {
    configured.map_or_else(
        || TranscriptionResponse {
            text: None,
            extra: Object::new(),
        },
        Clone::clone,
    )
}
