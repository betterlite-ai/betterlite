use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_config::types::{Config, ProxyTrust};
use betterlite_core::api::file_http_metadata::config::MetadataConfig;
use betterlite_core::api::file_http_metadata::error::MetadataError;
use betterlite_core::api::file_http_metadata::service::{
    MetadataCall, MetadataDeps, MetadataValue, prepare, send,
};
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_core::api::vector_store_http_write::service::{
    ClientKind, ClientSource, LogContext, WriteTransport,
};
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
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};

use super::super::error::SdkHttpError;

// The pooled client the selected deployment would get; the case env only carries the key and the proxy settings
pub(super) struct Source {
    pool: ClientPool,
    tls: TlsPolicy,
    routing: Arc<ProxyPolicy>,
}

impl ClientSource for Source {
    type Client = ProxiedClient;

    fn client(&self, _ssl_verify: Option<bool>) -> Result<ProxiedClient, ClientError> {
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

// The pre-call and the sends a production request runs: one logging state, one environment, from prepare to the reply
pub(super) struct Fixture {
    redactor: Redactor,
    sink: Sink,
    state: LoggingState,
    now: DateTime<FixedOffset>,
}

impl Fixture {
    pub(super) fn new(call_type: &str) -> Result<Self, SdkHttpError> {
        let text = format!(
            r#"{{"model":"files","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
        );
        let init: LoggingInit = serde_json::from_str(&text)
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

    pub(super) async fn execute<C, S>(
        &mut self,
        deps: &MetadataDeps<'_, C, S>,
        mode: ClientKind,
        call: &MetadataCall<'_>,
    ) -> Result<MetadataValue, MetadataError>
    where
        C: MetadataConfig,
        S: ClientSource<Client: WriteTransport>,
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
        let prepared = prepare(deps, mode, call, &mut log)?;
        send(prepared, &mut log).await
    }
}
