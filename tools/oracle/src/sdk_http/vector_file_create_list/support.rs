use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_config::types::{Config, ProxyTrust};
use betterlite_core::api::vector_file_http_create_list::service::ClientSource;
use betterlite_core::api::vector_file_provider::{
    CredentialLookup, CredentialLookupError, OperationKwargs,
};
use betterlite_core::api::vector_store_http_write::error::ClientError;
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
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

use super::super::error::SdkHttpError;
use super::Registry;

pub(super) const API_BASE_SENTINEL: &str = "$API_BASE";

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
pub(super) struct Assets {
    pub(super) redactor: Redactor,
    now: DateTime<FixedOffset>,
}
impl Assets {
    pub(super) fn new() -> Result<Self, SdkHttpError> {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
        let now = moment()?;
        Ok(Self { redactor, now })
    }

    pub(super) fn env(&self) -> PrePostEnv<'_> {
        PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: self,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: self.now,
        }
    }
}

impl LogSink for Assets {
    fn emit(&self, _record: LogRecord) {}
}

fn moment() -> Result<DateTime<FixedOffset>, SdkHttpError> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))
}

pub(super) fn logging_state(call_type: &str) -> Result<LoggingState, SdkHttpError> {
    let text = format!(
        r#"{{"model":"","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
    );
    let init: LoggingInit =
        serde_json::from_str(&text).map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated"),
        Duration::default,
    )
    .map_err(SdkHttpError::LoggingState)
}

pub(super) struct CaseRegistry {
    vector_store_id: String,
    values: IndexMap<String, Json>,
    failing: bool,
}

impl CredentialLookup for CaseRegistry {
    fn lookup_credentials(
        &self,
        vector_store_id: &str,
    ) -> Result<OperationKwargs, CredentialLookupError> {
        if self.failing {
            return Err(CredentialLookupError::Unavailable {
                reason: String::from("the case registry raised"),
            });
        }
        if vector_store_id != self.vector_store_id {
            return Ok(OperationKwargs(IndexMap::new()));
        }
        Ok(OperationKwargs(self.values.clone()))
    }
}

pub(super) fn registry_of(registry: &Registry, api_base: &str) -> CaseRegistry {
    let values = registry
        .credential_values
        .iter()
        .map(|(name, value)| {
            let value = match value {
                Json::Text(text) if text.as_str() == API_BASE_SENTINEL => {
                    Json::Text(api_base.to_owned())
                }
                other => other.clone(),
            };
            (name.clone(), value)
        })
        .collect();
    CaseRegistry {
        vector_store_id: registry.vector_store_id.clone(),
        values,
        failing: registry.failing,
    }
}
