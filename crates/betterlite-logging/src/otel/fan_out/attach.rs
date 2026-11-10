use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::otel::config::destination::{OtelDestination, SpanScope};
use crate::otel::config::{ExporterOwner, ExporterSpec, Settings, Signal};
use crate::otel::fan_out::destination::{DestinationProcessorFactory, ExporterDestinationFactory};
use crate::otel::fan_out::drain::{MAX_PENDING_DRAINS, SHUTDOWN_DRAIN};
use crate::otel::fan_out::processor::{FanOutSettings, TenantFanOutSpanProcessor};
use crate::otel::fan_out::scope::RequestScopeSource;
use crate::otel::plumbing::endpoints::signal_endpoint;
use crate::otel::plumbing::exporters::{ExporterFactories, Transport};
use crate::otel::plumbing::headers::parse_headers;

// An exporter's account: its normalized endpoint and the credentials it presents.
pub type SinkKey = (String, Vec<(String, String)>);

// Header names that spell one credential two ways: Arize's exporter sends space_id, a tenant arize-space-id.
const CREDENTIAL_ALIASES: [(&str, &str); 1] = [("arize_space_id", "space_id")];

// The processors attached to the one provider every span passes through; the fan-out lives here.
#[derive(Debug, Default)]
pub struct TenantFanOutRegistry {
    pub fan_out: Option<TenantFanOutSpanProcessor>,
}

// litellm.integrations.otel.plumbing.providers._credential_name
#[must_use]
pub fn credential_name(header: &str) -> String {
    let normalized = header.trim().to_lowercase().replace('-', "_");
    match CREDENTIAL_ALIASES
        .iter()
        .find(|(alias, _canonical)| normalized == *alias)
    {
        Some((_alias, canonical)) => (*canonical).to_owned(),
        None => normalized,
    }
}

// litellm.integrations.otel.plumbing.providers._sink_key
#[must_use]
pub fn sink_key(endpoint: Option<&str>, headers: &BTreeMap<String, String>) -> Option<SinkKey> {
    let normalized = signal_endpoint(endpoint, Signal::Traces)?;
    let mut credentials: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| (credential_name(name), value.clone()))
        .collect();
    credentials.sort();
    Some((normalized, credentials))
}

// litellm.integrations.otel.plumbing.providers.operator_sink_scopes
#[must_use]
pub fn operator_sink_scopes(
    configs: &[Settings],
    factories: &ExporterFactories,
) -> BTreeMap<SinkKey, SpanScope> {
    let mut scoped = BTreeMap::new();
    for config in configs {
        for spec in &config.exporters {
            if !exports_to_the_wire(spec, factories) {
                continue;
            }
            let headers = parse_headers(spec.headers.as_deref());
            let Some(key) = sink_key(spec.endpoint.as_deref(), &headers) else {
                continue;
            };
            let scope = operator_scope(config, spec);
            let widest = scoped.entry(key).or_insert(scope);
            if scope == SpanScope::Full {
                *widest = SpanScope::Full;
            }
        }
    }
    scoped
}

// litellm.integrations.otel.plumbing.providers.attach_tenant_fan_out; the bool tells a repeat attach from the first
#[must_use]
pub fn attach_tenant_fan_out(
    registry: &mut TenantFanOutRegistry,
    factories: Arc<ExporterFactories>,
    scope: Arc<dyn RequestScopeSource>,
    configs: &[Settings],
    excluded_db_systems: BTreeSet<String>,
) -> bool {
    if registry.fan_out.is_some() {
        return false;
    }
    let operator_sinks = operator_sink_scopes(configs, &factories);
    let build: Arc<dyn DestinationProcessorFactory> =
        Arc::new(ExporterDestinationFactory { factories });
    let settings = FanOutSettings {
        operator_sinks,
        excluded_db_systems,
        shutdown_drain: SHUTDOWN_DRAIN,
        pending_drains: MAX_PENDING_DRAINS,
        drain_pool: None,
    };
    registry.fan_out = Some(TenantFanOutSpanProcessor::new(scope, build, settings));
    true
}

// litellm.integrations.otel.plumbing.providers.deliverable_destinations
#[must_use]
pub fn deliverable_destinations(
    destinations: &[OtelDestination],
    registry: &TenantFanOutRegistry,
) -> Vec<OtelDestination> {
    registry
        .fan_out
        .as_ref()
        .map_or_else(Vec::new, |fan_out| fan_out.deliverable(destinations))
}

// litellm.integrations.otel.plumbing.providers._operator_scope
fn operator_scope(config: &Settings, spec: &ExporterSpec) -> SpanScope {
    if spec.owner == Some(ExporterOwner::LangfuseOtel) {
        config.langfuse_span_scope
    } else {
        SpanScope::Full
    }
}

// litellm.integrations.otel.plumbing.providers._exports_to_the_wire
fn exports_to_the_wire(spec: &ExporterSpec, factories: &ExporterFactories) -> bool {
    factories.exporter_transport(&spec.kind) != Transport::Headerless
        && !(spec.requires_headers && spec.headers.is_none())
}

#[path = "attach/tests.rs"]
mod tests;
