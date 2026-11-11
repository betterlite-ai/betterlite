use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::SecretString;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::image_call::plan::CredentialSource;
use crate::image_call::transport::{
    ClientFactory, ImageReply, ImageRequest, ImageTransport, TransferError,
};

// 1_700_000_000 seconds is 2023-11-14T22:13:20Z, the instant the signing assertions name
pub(super) fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .unwrap()
}

pub(super) type Seen = Arc<Mutex<Vec<ImageRequest>>>;

#[derive(Clone, Debug)]
pub(super) enum Reply {
    Answer(ImageReply),
    Timeout,
    Failed,
}

impl Reply {
    pub(super) fn ok(body: &str) -> Self {
        Self::Answer(ImageReply {
            status: 200,
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
        })
    }

    pub(super) fn status(status: u16, headers: &[(&str, &str)], body: &str) -> Self {
        Self::Answer(ImageReply {
            status,
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            body: body.as_bytes().to_vec(),
        })
    }
}

// the fixture client stands in for both an HTTPHandler and an AsyncHTTPHandler and records the sent request
pub(super) struct FixtureClient {
    pub(super) reply: Reply,
    pub(super) seen: Seen,
}

impl FixtureClient {
    pub(super) fn new(reply: Reply) -> Self {
        Self {
            reply,
            seen: Seen::default(),
        }
    }

    pub(super) fn requests(&self) -> Vec<ImageRequest> {
        self.seen.lock().unwrap().clone()
    }
}

impl ImageTransport for FixtureClient {
    fn post(
        &self,
        request: ImageRequest,
    ) -> impl Future<Output = Result<ImageReply, TransferError>> + Send {
        self.seen.lock().unwrap().push(request);
        let reply = match &self.reply {
            Reply::Answer(reply) => Ok(reply.clone()),
            Reply::Timeout => Err(TransferError::Timeout),
            Reply::Failed => Err(TransferError::Failed(TransportError::Closed)),
        };
        ready(reply)
    }
}

pub(super) struct Factory {
    pub(super) reply: Reply,
    pub(super) sync_calls: Mutex<u32>,
    pub(super) async_calls: Mutex<Vec<Option<Duration>>>,
    pub(super) seen: Seen,
}

impl Factory {
    pub(super) fn new(reply: Reply) -> Self {
        Self {
            reply,
            sync_calls: Mutex::new(0),
            async_calls: Mutex::new(Vec::new()),
            seen: Seen::default(),
        }
    }

    pub(super) fn seen_requests(&self) -> Vec<ImageRequest> {
        self.seen.lock().unwrap().clone()
    }
}

impl ClientFactory for Factory {
    type Client = FixtureClient;

    fn sync_client(&self) -> Self::Client {
        let mut calls = self.sync_calls.lock().unwrap();
        *calls = calls.saturating_add(1);
        drop(calls);
        FixtureClient {
            reply: self.reply.clone(),
            seen: Arc::clone(&self.seen),
        }
    }

    fn async_client(&self, timeout: Option<Duration>) -> Self::Client {
        self.async_calls.lock().unwrap().push(timeout);
        FixtureClient {
            reply: self.reply.clone(),
            seen: Arc::clone(&self.seen),
        }
    }
}

pub(super) struct Source {
    pub(super) credentials: Credentials,
    pub(super) calls: Mutex<Vec<(AwsSettings, String)>>,
}

impl Source {
    pub(super) fn new() -> Self {
        Self::with(
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        )
    }

    pub(super) fn with(access_key_id: &str, secret_access_key: &str) -> Self {
        Self {
            credentials: Credentials {
                access_key_id: SecretString::from(access_key_id.to_owned()),
                secret_access_key: SecretString::from(secret_access_key.to_owned()),
                session_token: None,
            },
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl CredentialSource for Source {
    fn resolve(
        &self,
        settings: AwsSettings,
        region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        self.calls.lock().unwrap().push((settings, region));
        ready(Ok(self.credentials.clone()))
    }
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap_or_default()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

#[derive(Default)]
pub(super) struct DiscardingSink;

impl LogSink for DiscardingSink {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) fn logging_state() -> LoggingState {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"amazon.titan-image-generator-v2:0","call_type":"image_generation","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .expect("the logging fixture is valid");
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor builds");
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || "generated".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds")
}

pub(super) fn prepost_env<'env>(
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

pub(super) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor builds")
}
