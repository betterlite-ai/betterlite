use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use aws_smithy_async::time::{SharedTimeSource, StaticTimeSource};
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::bedrock_embedding_io::credentials::ManagedRegionWorker;
use betterlite_bedrock::bedrock_embedding_io::error::EmbeddingIoError;
use betterlite_bedrock::embedding_call::Clock;
use betterlite_bedrock::signing::executor::AwsSigningExecutor;
use betterlite_bedrock::transcription::call::NativeTranscription;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState, LoggingStateError};
use betterlite_openai::api::audio_file_input::AuthorizedAudioFiles;
use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use betterlite_tokenizer::engine::error::EngineError;
use secrecy::SecretString;
use thiserror::Error;

pub mod binding;
pub mod client;
pub mod factory;
pub mod record;
pub mod source;

// 1_700_000_000 seconds is 2023-11-14T22:13:20Z, the instant the composition fixtures sign at
pub const NOW_SECONDS: i64 = 1_700_000_000;

#[derive(Debug, Error)]
pub enum FixtureError {
    #[error("the fixture instant is outside the clock")]
    Instant,
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Redact(#[from] RedactionError),
    #[error(transparent)]
    RegionWorker(#[from] EmbeddingIoError),
}

pub fn instant(seconds: i64) -> Result<SystemTime, FixtureError> {
    let whole = u64::try_from(seconds).map_err(|_error| FixtureError::Instant)?;
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(whole))
        .ok_or(FixtureError::Instant)
}

// The credentials a probe's optional_params carry are the identity its source resolves
#[must_use]
pub fn credentials(access_key_id: &str, secret_access_key: &str) -> Credentials {
    Credentials {
        access_key_id: SecretString::from(access_key_id.to_owned()),
        secret_access_key: SecretString::from(secret_access_key.to_owned()),
        session_token: None,
    }
}

// The clock every composition row signs at; a family that reads the clock twice sees the same instant
pub struct FixedClock(pub SystemTime);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

// The encoding the embedding leaf reads is only asked for a count, and its fixture counts characters
pub struct Counting;

impl TextEncoding for Counting {
    fn count_tokens(&self, text: &str) -> Result<usize, EngineError> {
        Ok(text.chars().count())
    }
}

#[derive(Default)]
pub struct DiscardingSink;

impl LogSink for DiscardingSink {
    fn emit(&self, _record: LogRecord) {}
}

pub struct Files;

impl AuthorizedAudioFiles for Files {
    fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::from(io::ErrorKind::NotFound))
    }

    fn open(&self, _path: &Path) -> io::Result<Box<dyn Read + Send>> {
        Err(io::Error::from(io::ErrorKind::NotFound))
    }
}

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: String::from("the fixture has no OIDC transport"),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoOidc {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

#[must_use]
pub fn secret_lookup() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub fn logging_state(redactor: &Redactor) -> Result<LoggingState, FixtureError> {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"amazon.titan-embed-text-v1","call_type":"embedding","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )?;
    Ok(LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "generated".to_owned(),
        Duration::default,
    )?)
}

// The native transcription binding the composition reaches, over the fixture transport and a fixed clock
#[must_use]
pub fn native_binding(
    transport: client::FixtureClient,
    credentials: source::Source,
    environment: BTreeMap<String, String>,
) -> NativeTranscription<client::FixtureClient, source::Source> {
    NativeTranscription {
        transport,
        credentials,
        environment,
        clock: SharedTimeSource::from(StaticTimeSource::from_secs(
            u64::try_from(NOW_SECONDS).unwrap_or_default(),
        )),
    }
}

// The dependencies one dispatch reads that do not come from its transport, built once per exchange
pub struct Prepared {
    pub redactor: Redactor,
    pub state: LoggingState,
    pub secrets: SecretLookup,
    pub workers: ManagedRegionWorker,
    pub signing: AwsSigningExecutor,
    pub encoding: Counting,
    pub files: Files,
    pub clock: Arc<dyn Clock>,
}

impl Prepared {
    pub fn new() -> Result<Self, FixtureError> {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
        let state = logging_state(&redactor)?;
        Ok(Self {
            redactor,
            state,
            secrets: secret_lookup(),
            workers: ManagedRegionWorker::new()?,
            signing: AwsSigningExecutor::default(),
            encoding: Counting,
            files: Files,
            clock: Arc::new(FixedClock(instant(NOW_SECONDS)?)),
        })
    }
}

#[must_use]
pub fn prepost_env<'env>(
    state: &LoggingState,
    redactor: &'env Redactor,
    sink: &'env DiscardingSink,
) -> PrePostEnv<'env> {
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
        now: state.start_time,
    }
}
