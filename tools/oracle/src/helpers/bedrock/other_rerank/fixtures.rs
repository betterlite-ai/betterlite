use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::rerank::plan::CredentialSource;
use betterlite_bedrock::rerank::transport::{
    ClientFactory, RerankHttpReply, RerankHttpRequest, RerankTransport, TransferError,
};
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

use super::driver::Failure;
use super::input::{Probe, Reply, ReplyFailure};

type Seen = Arc<Mutex<Vec<(RerankHttpRequest, bool)>>>;

#[derive(Clone)]
pub(super) struct Client {
    reply: Reply,
    seen: Seen,
}

impl Client {
    pub(super) fn new(reply: &Reply) -> Self {
        Self {
            reply: reply.clone(),
            seen: Seen::default(),
        }
    }

    pub(super) fn requests(&self) -> Vec<(RerankHttpRequest, bool)> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn answer(
        &self,
        request: RerankHttpRequest,
        with_logging: bool,
    ) -> Result<RerankHttpReply, TransferError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((request, with_logging));
        match self.reply.failure {
            None => Ok(RerankHttpReply {
                status: self.reply.status,
                headers: self.reply.headers.clone().into_iter().collect(),
                body: self.reply.body.as_bytes().to_vec(),
            }),
            Some(ReplyFailure::Timeout) => Err(TransferError::Timeout),
            Some(ReplyFailure::Failed) => Err(TransferError::Failed(TransportError::Closed)),
        }
    }
}

impl RerankTransport for Client {
    fn post(
        &self,
        request: RerankHttpRequest,
    ) -> impl Future<Output = Result<RerankHttpReply, TransferError>> + Send {
        ready(self.answer(request, false))
    }

    fn post_with_logging(
        &self,
        request: RerankHttpRequest,
        _logging: &LoggingState,
    ) -> impl Future<Output = Result<RerankHttpReply, TransferError>> + Send {
        ready(self.answer(request, true))
    }
}

impl ClientFactory for Client {
    type Client = Self;

    fn sync_client(&self) -> Self::Client {
        self.clone()
    }

    fn async_client(&self) -> Self::Client {
        self.clone()
    }
}

pub(super) struct Source {
    credentials: Credentials,
}

impl Source {
    pub(super) fn new(probe: &Probe) -> Self {
        Self {
            credentials: Credentials {
                access_key_id: SecretString::from(probe.credentials.access_key_id.clone()),
                secret_access_key: SecretString::from(probe.credentials.secret_access_key.clone()),
                session_token: probe
                    .credentials
                    .session_token
                    .as_ref()
                    .map(|token| SecretString::from(token.clone())),
            },
        }
    }
}

impl CredentialSource for Source {
    fn resolve(
        &self,
        _settings: AwsSettings,
        _region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        ready(Ok(self.credentials.clone()))
    }
}

#[derive(Default)]
pub(super) struct DiscardingSink;

impl LogSink for DiscardingSink {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) fn logging_state() -> Result<LoggingState, Failure> {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"rerank-model","call_type":"rerank","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )?;
    let redactor = redactor()?;
    Ok(LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || "generated".to_owned(),
        Duration::default,
    )?)
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

pub(super) fn redactor() -> Result<Redactor, Failure> {
    Ok(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?)
}
