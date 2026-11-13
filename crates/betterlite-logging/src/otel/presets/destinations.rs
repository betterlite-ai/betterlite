use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_http::url_policy::destination::destination_allowed;
use betterlite_models::prompt_contracts::StandardCallbackDynamicParams;

use crate::langfuse_otel::cloud_endpoints::LANGFUSE_CLOUD_US_ENDPOINT;
use crate::langfuse_otel::destination::langfuse_otel_host;
use crate::otel::config::destination::{DestinationFields, OtelDestination, SpanScope};
use crate::otel::integrations::arize_settings::ArizeConfig;
use crate::otel::integrations::weave_settings::weave_otel_endpoint;
use crate::otel::presets::newrelic::newrelic_dynamic_endpoint;
use crate::otel::presets::{dynamic_otlp_headers, dynamic_params_map};

// A backend is destination capable only when it builds tenant headers too, else the destination would carry no credentials
const DESTINATION_HEADERS: &[(&str, &[&str])] = &[
    ("arize", &["arize-space-id", "api_key"]),
    ("langfuse_otel", &["Authorization"]),
    ("newrelic", &["api-key"]),
    ("weave_otel", &["Authorization", "project_id"]),
];

#[must_use]
pub fn destination_capable_backends() -> BTreeSet<&'static str> {
    let mut backends = BTreeSet::new();
    for (callback_name, _required) in DESTINATION_HEADERS {
        backends.insert(*callback_name);
    }
    backends
}

#[must_use]
pub fn destination_for(
    environment: &dyn EnvironmentLookup,
    allowed_hosts: &[AllowedDestination],
    callback_name: &str,
    params: &StandardCallbackDynamicParams,
    service_name: Option<&str>,
) -> Option<OtelDestination> {
    let required = DESTINATION_HEADERS
        .iter()
        .find_map(|(name, required)| (*name == callback_name).then_some(*required))?;
    let values = dynamic_params_map(params);
    let headers = dynamic_otlp_headers(Some(callback_name), params, allowed_hosts)?;
    if !required.iter().all(|name| headers.contains_key(*name)) {
        return None;
    }
    let (endpoint, protocol) =
        endpoint_and_protocol(environment, allowed_hosts, callback_name, &values)?;
    let mut resource_attributes = BTreeMap::new();
    if let Some(configured_name) = service_name.filter(|name| !name.is_empty()) {
        resource_attributes.insert("service.name".to_owned(), configured_name.to_owned());
    }
    let span_scope = if callback_name == "langfuse_otel"
        && values
            .get("langfuse_span_scope")
            .is_some_and(|scope| scope == "llm_only")
    {
        SpanScope::LlmOnly
    } else {
        SpanScope::Full
    };
    Some(OtelDestination::from(DestinationFields {
        endpoint,
        headers,
        resource_attributes,
        callback_name: Some(callback_name.to_owned()),
        protocol,
        span_scope,
    }))
}

fn endpoint_and_protocol(
    environment: &dyn EnvironmentLookup,
    allowed_hosts: &[AllowedDestination],
    callback_name: &str,
    params: &BTreeMap<String, String>,
) -> Option<(String, Option<String>)> {
    match callback_name {
        "langfuse_otel" => Some((
            langfuse_destination(environment, allowed_hosts, params)?,
            None,
        )),
        "arize" => {
            let config = ArizeConfig::from_env(environment);
            Some((
                config.endpoint,
                Some(config.protocol.exporter_kind().to_owned()),
            ))
        }
        "weave_otel" => Some((
            weave_otel_endpoint(environment.lookup("WANDB_HOST").as_deref()),
            None,
        )),
        "newrelic" => Some((newrelic_dynamic_endpoint(params), None)),
        _ => None,
    }
}

// The tenant's own host has to be allowlisted; the operator's own LANGFUSE_HOST is a deployment choice
fn langfuse_destination(
    environment: &dyn EnvironmentLookup,
    allowed_hosts: &[AllowedDestination],
    params: &BTreeMap<String, String>,
) -> Option<String> {
    let tenant_host = params
        .get("langfuse_host")
        .filter(|value| !value.is_empty())
        .cloned();
    let Some(host) = tenant_host
        .clone()
        .or_else(|| langfuse_host_from(environment))
    else {
        return Some(LANGFUSE_CLOUD_US_ENDPOINT.to_owned());
    };
    let normalized = if host.starts_with("http") {
        host.clone()
    } else {
        format!("https://{host}")
    };
    let endpoint = format!("{}/api/public/otel", normalized.trim_end_matches('/'));
    if tenant_host.is_some() && !destination_allowed(&endpoint, allowed_hosts) {
        tracing::warn!(
            "OTel V2: not exporting to key/team Langfuse host '{host}'. Add it to litellm_settings.provider_url_destination_allowed_hosts to permit it"
        );
        return None;
    }
    Some(endpoint)
}

// The Langfuse host resolver reads a snapshot, so only the two host names cross the lookup boundary
fn langfuse_host_from(environment: &dyn EnvironmentLookup) -> Option<String> {
    let mut values = BTreeMap::new();
    for name in ["LANGFUSE_OTEL_HOST", "LANGFUSE_HOST"] {
        if let Some(value) = environment.lookup(name) {
            values.insert(name.to_owned(), value);
        }
    }
    langfuse_otel_host(&values)
}

mod tests;
