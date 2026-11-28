use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_core::api::file_http_upload::config::Headers;
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_core::api::vector_store_http_write::service::ClientSource;
use betterlite_http::proxy::ProxiedClient;
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
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;

use super::super::super::error::SdkHttpError;
use super::super::Call;
use super::INIT;

pub(super) struct Source {
    pool: ClientPool,
    tls: TlsPolicy,
    routing: Arc<ProxyPolicy>,
}

impl Source {
    pub(super) fn new() -> Self {
        Self {
            pool: ClientPool::new(1, TransportLimits::DEFAULT),
            tls: TlsPolicy {
                verification: Verification::Enabled,
                ca_file: None,
                certificate: None,
                http2: false,
                ecdh_curve: None,
                user_agent: String::from("betterlite-oracle"),
            },
            routing: Arc::new(ProxyPolicy {
                no_proxy: String::new(),
                primary: BTreeMap::new(),
                fallback: BTreeMap::new(),
                warnings: Vec::new(),
            }),
        }
    }
}

impl ClientSource for Source {
    type Client = ProxiedClient;

    fn client(&self, _ssl_verify: Option<bool>) -> Result<ProxiedClient, ClientError> {
        self.pool
            .client(&self.tls, &self.routing)
            .map_err(ClientError::Pool)
    }
}

pub(super) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) fn fixture_now() -> Result<DateTime<FixedOffset>, SdkHttpError> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))
}

pub(super) fn logging_state(redactor: &Redactor) -> Result<LoggingState, SdkHttpError> {
    let init: LoggingInit =
        serde_json::from_str(INIT).map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
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
    .map_err(SdkHttpError::LoggingState)
}

pub(super) fn fixture_env<'env>(
    redactor: &'env Redactor,
    sink: &'env Sink,
    now: DateTime<FixedOffset>,
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
        now,
    }
}

pub(super) fn base_headers(call: &Call) -> Headers {
    call.headers
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.as_str())))
        .collect()
}
