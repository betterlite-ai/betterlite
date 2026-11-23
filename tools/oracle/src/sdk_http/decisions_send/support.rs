use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_redact::secret::Redactor;
use chrono::DateTime;

use super::super::error::SdkHttpError;

// 2026-01-01T00:00:05+00:00, the fixed instant every logging record in a case is stamped with
pub(super) const FIXTURE_NOW_NANOS: i64 = 1_767_225_605_000_000_000;

// The case configures no callbacks, so a logging sink that keeps nothing is what pre_call reports to
pub(super) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// The pool the handler draws both clients from; the case env carries any proxy settings
pub(super) fn clients(environment: &BTreeMap<String, String>) -> Result<HttpClients, SdkHttpError> {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), environment)
        .map_err(SdkHttpError::ProxyPolicy)?;
    Ok(HttpClients::new(
        TlsPolicy {
            verification: Verification::Enabled,
            ca_file: None,
            certificate: None,
            http2: false,
            ecdh_curve: None,
            user_agent: "betterlite-oracle/0".to_owned(),
        },
        Arc::new(policy),
    ))
}

// The logging environment send sees: no logging object means pre_call never runs
pub(super) fn prepost_env<'env>(
    redactor: &'env Redactor,
    log: &'env dyn LogSink,
) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::from_timestamp_nanos(FIXTURE_NOW_NANOS).fixed_offset(),
    }
}
