use std::future::{Future, ready};
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

use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::rerank::plan::CredentialSource;
use crate::rerank::transport::{
    ClientFactory, RerankHttpReply, RerankHttpRequest, RerankTransport, TransferError,
};

// 1_700_000_000 seconds is 2023-11-14T22:13:20Z, the instant the signing assertions name
pub(super) fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .unwrap()
}

pub(super) type Seen = Arc<Mutex<Vec<(RerankHttpRequest, bool)>>>;

#[derive(Clone, Debug)]
pub(super) enum Reply {
    Answer(RerankHttpReply),
    Timeout,
    Failed,
}

impl Reply {
    pub(super) fn ok(body: &str) -> Self {
        Self::status(200, &[], body)
    }

    pub(super) fn status(status: u16, headers: &[(&str, &str)], body: &str) -> Self {
        Self::Answer(RerankHttpReply {
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

    pub(super) fn requests(&self) -> Vec<(RerankHttpRequest, bool)> {
        self.seen.lock().unwrap().clone()
    }

    fn record(&self, request: &RerankHttpRequest, with_logging: bool) {
        self.seen
            .lock()
            .unwrap()
            .push((request.clone(), with_logging));
    }

    fn reply(&self) -> Result<RerankHttpReply, TransferError> {
        match &self.reply {
            Reply::Answer(reply) => Ok(reply.clone()),
            Reply::Timeout => Err(TransferError::Timeout),
            Reply::Failed => Err(TransferError::Failed(TransportError::Closed)),
        }
    }
}

impl RerankTransport for FixtureClient {
    fn post(
        &self,
        request: RerankHttpRequest,
    ) -> impl Future<Output = Result<RerankHttpReply, TransferError>> + Send {
        self.record(&request, false);
        ready(self.reply())
    }

    fn post_with_logging(
        &self,
        request: RerankHttpRequest,
        _logging: &LoggingState,
    ) -> impl Future<Output = Result<RerankHttpReply, TransferError>> + Send {
        self.record(&request, true);
        ready(self.reply())
    }
}

pub(super) struct Factory {
    pub(super) reply: Reply,
    pub(super) sync_calls: Mutex<u32>,
    pub(super) async_calls: Mutex<u32>,
    pub(super) seen: Seen,
}

impl Factory {
    pub(super) fn new(reply: Reply) -> Self {
        Self {
            reply,
            sync_calls: Mutex::new(0),
            async_calls: Mutex::new(0),
            seen: Seen::default(),
        }
    }

    pub(super) fn requests(&self) -> Vec<(RerankHttpRequest, bool)> {
        self.seen.lock().unwrap().clone()
    }

    fn client(&self) -> FixtureClient {
        FixtureClient {
            reply: self.reply.clone(),
            seen: Arc::clone(&self.seen),
        }
    }
}

impl ClientFactory for Factory {
    type Client = FixtureClient;

    fn sync_client(&self) -> Self::Client {
        let mut calls = self.sync_calls.lock().unwrap();
        *calls = calls.saturating_add(1);
        drop(calls);
        self.client()
    }

    fn async_client(&self) -> Self::Client {
        let mut calls = self.async_calls.lock().unwrap();
        *calls = calls.saturating_add(1);
        drop(calls);
        self.client()
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

#[derive(Default)]
pub(super) struct DiscardingSink;

impl LogSink for DiscardingSink {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) fn logging_state() -> LoggingState {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"rerank-model","call_type":"rerank","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
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
