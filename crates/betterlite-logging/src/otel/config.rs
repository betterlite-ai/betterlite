use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::otel_settings;
use betterlite_config::tls::{TlsPolicy, TlsPolicyError, Verification};
use betterlite_config::types::Config;
use tracing::warn;

use crate::otel::vocabulary::resolvers::db_system;

pub mod destination;
use destination::SpanScope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMessageContent {
    NoContent,
    SpanOnly,
    EventOnly,
    SpanAndEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExporterOwner {
    ArizeAx,
    ArizePhoenix,
    LangfuseOtel,
    WeaveOtel,
    Levo,
    Agentops,
    Newrelic,
    Signoz,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExporterSpec {
    pub kind: String,
    pub endpoint: Option<String>,
    pub traces_endpoint: Option<String>,
    pub headers: Option<String>,
    pub owner: Option<ExporterOwner>,
    pub options: Option<BTreeMap<String, String>>,
    pub use_simple_processor: Option<bool>,
    pub requires_headers: bool,
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "OTel exposes independent feature switches"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub exporters: Vec<ExporterSpec>,
    pub service_name: String,
    pub deployment_environment: Option<String>,
    pub enable_metrics: bool,
    pub enable_events: bool,
    pub capture_message_content: CaptureMessageContent,
    pub legacy_compat: bool,
    pub langfuse_span_scope: SpanScope,
    pub excluded_services: BTreeSet<String>,
    pub mapper_names: Vec<String>,
    pub resource_attributes: BTreeMap<String, String>,
    pub baggage_promoted_keys: Vec<String>,
    pub baggage_metadata_keys: Vec<String>,
    pub baggage_team_metadata_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OtlpHttpTls {
    NoVerify,
    CertificateFile(String),
    DefaultVerified,
}

#[derive(Debug, thiserror::Error)]
pub enum OTelConfigError {
    #[error("unknown capture_message_content value {0}")]
    CaptureMode(String),
    #[error("unknown langfuse_span_scope value {0}")]
    SpanScope(String),
    #[error("unknown exporter owner {0}")]
    ExporterOwner(String),
    #[error(transparent)]
    Tls(#[from] TlsPolicyError),
}

impl Settings {
    pub fn from_config(config: &Config) -> Result<Self, OTelConfigError> {
        let raw = &config.sections.otel_settings;
        let capture_message_content = capture_mode(&raw.capture_message_content)?;
        let langfuse_span_scope = span_scope(&raw.langfuse_span_scope)?;
        let excluded_services = excluded_services(&raw.excluded_services);
        let exporters = exporter_specs(&raw.exporters)?;
        Ok(Self {
            exporters,
            service_name: raw.service_name.clone(),
            deployment_environment: raw.deployment_environment.clone(),
            enable_metrics: raw.enable_metrics,
            enable_events: raw.enable_events,
            capture_message_content,
            legacy_compat: raw.legacy_compat,
            langfuse_span_scope,
            excluded_services,
            mapper_names: raw.mapper_names.clone(),
            resource_attributes: raw.resource_attributes.clone(),
            baggage_promoted_keys: raw.baggage_promoted_keys.clone(),
            baggage_metadata_keys: raw.baggage_metadata_keys.clone(),
            baggage_team_metadata_keys: raw.baggage_team_metadata_keys.clone(),
        })
    }

    #[must_use]
    pub const fn capture_span_content(&self) -> bool {
        matches!(
            self.capture_message_content,
            CaptureMessageContent::SpanOnly | CaptureMessageContent::SpanAndEvent
        )
    }
}

fn capture_mode(value: &str) -> Result<CaptureMessageContent, OTelConfigError> {
    match value {
        "no_content" => Ok(CaptureMessageContent::NoContent),
        "span_only" => Ok(CaptureMessageContent::SpanOnly),
        "event_only" => Ok(CaptureMessageContent::EventOnly),
        "span_and_event" => Ok(CaptureMessageContent::SpanAndEvent),
        unknown => Err(OTelConfigError::CaptureMode(unknown.to_owned())),
    }
}

fn span_scope(value: &str) -> Result<SpanScope, OTelConfigError> {
    match value {
        "full" => Ok(SpanScope::Full),
        "llm_only" => Ok(SpanScope::LlmOnly),
        unknown => Err(OTelConfigError::SpanScope(unknown.to_owned())),
    }
}

fn excluded_services(services: &[String]) -> BTreeSet<String> {
    services
        .iter()
        .filter_map(|service| {
            let name = service.trim().to_ascii_lowercase();
            let system = if name == "postgresql" {
                Some("postgresql")
            } else {
                db_system(&name)
            };
            if system.is_none() {
                warn!(service = %service, "ignoring unknown OpenTelemetry excluded service");
            }
            system.map(str::to_owned)
        })
        .collect()
}

fn exporter_specs(
    raw_specs: &[otel_settings::ExporterSpec],
) -> Result<Vec<ExporterSpec>, OTelConfigError> {
    raw_specs
        .iter()
        .map(|spec| {
            Ok(ExporterSpec {
                kind: spec.kind.clone().unwrap_or_else(|| "console".to_owned()),
                endpoint: spec.endpoint.clone(),
                traces_endpoint: spec.traces_endpoint.clone(),
                headers: spec.headers.clone(),
                owner: spec.owner.as_deref().map(exporter_owner).transpose()?,
                options: spec.options.clone(),
                use_simple_processor: spec.use_simple_processor,
                requires_headers: spec.requires_headers.unwrap_or(false),
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
pub enum Signal {
    Traces,
    Metrics,
    Logs,
}

pub fn resolve_otlp_http_tls(
    config: &Config,
    environment: &BTreeMap<String, String>,
    signal: Signal,
) -> Result<OtlpHttpTls, OTelConfigError> {
    let signal_certificate = match signal {
        Signal::Traces => "OTEL_EXPORTER_OTLP_TRACES_CERTIFICATE",
        Signal::Metrics => "OTEL_EXPORTER_OTLP_METRICS_CERTIFICATE",
        Signal::Logs => "OTEL_EXPORTER_OTLP_LOGS_CERTIFICATE",
    };
    if ["OTEL_EXPORTER_OTLP_CERTIFICATE", signal_certificate]
        .iter()
        .any(|name| {
            environment
                .get(*name)
                .is_some_and(|value| !value.is_empty())
        })
    {
        return Ok(OtlpHttpTls::DefaultVerified);
    }
    let policy = TlsPolicy::resolve(config, environment, None)?;
    Ok(match policy.verification {
        Verification::Disabled => OtlpHttpTls::NoVerify,
        Verification::CaFile(path) => OtlpHttpTls::CertificateFile(path),
        Verification::Enabled => OtlpHttpTls::DefaultVerified,
    })
}

fn exporter_owner(value: &str) -> Result<ExporterOwner, OTelConfigError> {
    let owners = [
        ("arize", ExporterOwner::ArizeAx),
        ("arize_phoenix", ExporterOwner::ArizePhoenix),
        ("langfuse_otel", ExporterOwner::LangfuseOtel),
        ("weave_otel", ExporterOwner::WeaveOtel),
        ("levo", ExporterOwner::Levo),
        ("agentops", ExporterOwner::Agentops),
        ("newrelic", ExporterOwner::Newrelic),
        ("signoz", ExporterOwner::Signoz),
    ];
    owners
        .iter()
        .find_map(|(name, owner)| (*name == value).then_some(*owner))
        .ok_or_else(|| OTelConfigError::ExporterOwner(value.to_owned()))
}

#[path = "config/tests.rs"]
mod tests;
