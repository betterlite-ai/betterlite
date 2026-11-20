pub mod attributes;
pub mod env;
pub mod legacy;
pub mod maybe;
pub mod phoenix_auto;
pub mod shared;

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};

use betterlite_config::tls::Verification;
use betterlite_http::otlp_client::OtlpClient;
use betterlite_http::tls::websocket_client_config;
use opentelemetry_http::HttpClient;

use self::shared::SharedOtel;
use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::otel::config::{OtlpHttpTls, Settings, Signal, resolve_otlp_http_tls};
use crate::otel::legacy_policy::config::OpenTelemetryConfig;
use crate::otel::logger_runtime::LoggerRuntime;
use crate::otel::logger_runtime::factory::build_otel_v2_logger;
use crate::otel::logger_runtime::types::{ConstructorSettings, InjectedProviders};
use crate::otel::plumbing::exporters::HttpClients;

// LiteLLM's otel arm: the V2 logger when the switch is on, the legacy OpenTelemetry otherwise
pub fn build(
    inputs: &CatalogInputs,
    runtime: Option<&LoggerRuntime>,
    shared: &SharedOtel,
    in_memory: &mut Vec<Arc<dyn CallbackLogger>>,
) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    if maybe::v2_enabled(inputs)? {
        return build_v2(inputs, runtime, in_memory);
    }
    let config = legacy_config(inputs, shared);
    legacy::plain(legacy::prepare(inputs, shared, config, None)?)
}

// LiteLLM returns any OpenTelemetryV2 already in memory, then builds one from callback_settings.otel
fn build_v2(
    inputs: &CatalogInputs,
    runtime: Option<&LoggerRuntime>,
    in_memory: &mut Vec<Arc<dyn CallbackLogger>>,
) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let runtime = maybe::require_runtime(runtime, "otel")?;
    {
        let held = runtime
            .loggers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(existing) = held.first() {
            return Ok(Arc::clone(&existing.callback));
        }
    }
    let config = Settings::from_config(&inputs.config).map_err(|source| CatalogError::Init {
        message: source.to_string(),
    })?;
    let logger = build_otel_v2_logger(
        runtime,
        config,
        Some("otel"),
        InjectedProviders::default(),
        ConstructorSettings::default(),
    )
    .map_err(|source| CatalogError::Init {
        message: source.to_string(),
    })?;
    let callback = Arc::clone(&logger.callback);
    in_memory.push(Arc::clone(&callback));
    Ok(callback)
}

// OpenTelemetry(**callback_settings.otel) with no config reaches OpenTelemetryConfig.from_env(); the
// attribute filter is the one setting the constructor reads from the settings dict (opentelemetry.py 395)
fn legacy_config(inputs: &CatalogInputs, shared: &SharedOtel) -> OpenTelemetryConfig {
    let mut config = OpenTelemetryConfig::from_env(&shared.env);
    config.normalize(&shared.env);
    config.attributes = attributes::value(inputs.config.sections.otel_settings.attributes.as_ref());
    config
}

// One OTLP client per signal, over the pooled ProxyPolicy, with the TLS resolve_otlp_http_tls picks.
pub fn otlp_http_clients(inputs: &CatalogInputs) -> Result<HttpClients, CatalogError> {
    let environment = env::snapshot(inputs);
    Ok(HttpClients {
        traces: otlp_client(inputs, &environment, Signal::Traces)?,
        metrics: otlp_client(inputs, &environment, Signal::Metrics)?,
        logs: otlp_client(inputs, &environment, Signal::Logs)?,
    })
}

fn otlp_client(
    inputs: &CatalogInputs,
    environment: &BTreeMap<String, String>,
    signal: Signal,
) -> Result<Arc<dyn HttpClient>, CatalogError> {
    let wire = resolve_otlp_http_tls(&inputs.config, environment, signal).map_err(|source| {
        CatalogError::Init {
            message: source.to_string(),
        }
    })?;
    let verification = match wire {
        OtlpHttpTls::NoVerify => Verification::Disabled,
        OtlpHttpTls::CertificateFile(path) => Verification::CaFile(path),
        OtlpHttpTls::DefaultVerified => Verification::Enabled,
    };
    // websocket_client_config is the public bridge from a Verification to a rustls ClientConfig
    let tls = websocket_client_config(&verification).map_err(|source| CatalogError::Init {
        message: source.to_string(),
    })?;
    let client =
        OtlpClient::with_tls(tls, &inputs.http.policy).map_err(|source| CatalogError::Init {
            message: source.to_string(),
        })?;
    Ok(Arc::new(client))
}

pub(crate) mod tests;
