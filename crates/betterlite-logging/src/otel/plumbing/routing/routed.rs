use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::StandardCallbackDynamicParams;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use crate::otel::config::destination::SpanScope;
use crate::otel::config::{ExporterOwner, ExporterSpec, Settings};
use crate::otel::plumbing::routing::cache::TenantTracerCache;
use crate::otel::presets::{dynamic_otlp_endpoint, dynamic_otlp_headers, project_routing_headers};

// LiteLLM's OTEL_SERVICE_NAME_METADATA_KEYS, highest precedence first
const SERVICE_NAME_KEYS: [&str; 2] = ["otel_service_name_override", "otel_service_name"];

// exporter kinds that ignore headers, so a dynamic credential has nowhere to go (LiteLLM's list)
const NON_OTLP_KINDS: [&str; 4] = ["console", "in_memory", "inmemory", "memory"];

// project routing headers are honored on the OTLP/HTTP endpoint only (LiteLLM's list)
const GRPC_KINDS: [&str; 2] = ["otlp_grpc", "grpc"];

// urllib.parse.quote(value, safe=""): everything outside RFC 3986's unreserved set is escaped
const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

// litellm.integrations.otel.plumbing.routing.tenant_service_name: read only from the key/team config the proxy resolved at auth; it picks the dataset, so caller metadata never reaches it
#[must_use]
pub fn tenant_service_name(auth_metadata: Option<&BTreeMap<String, String>>) -> Option<String> {
    let resolved = auth_metadata?;
    SERVICE_NAME_KEYS
        .iter()
        .filter_map(|key| resolved.get(*key))
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .map(str::to_owned)
}

// the cache key LiteLLM sorts: (credential headers, project headers, endpoint, service name)
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RouteKey {
    pub credential_headers: Vec<(String, String)>,
    pub project_headers: Vec<(String, String)>,
    pub endpoint: Option<String>,
    pub service_name: Option<String>,
}

// the header values are collector credentials, so Debug prints only the header names, as CacheKey does
impl fmt::Debug for RouteKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let credentials: Vec<&str> = self
            .credential_headers
            .iter()
            .map(|(name, _value)| name.as_str())
            .collect();
        let projects: Vec<&str> = self
            .project_headers
            .iter()
            .map(|(name, _value)| name.as_str())
            .collect();
        f.debug_struct("RouteKey")
            .field("credential_headers", &credentials)
            .field("project_headers", &projects)
            .field("endpoint", &self.endpoint)
            .field("service_name", &self.service_name)
            .finish()
    }
}

// the request fields a route is keyed by; a separate trait keeps the cache to one inherent impl, and the lifecycle test crate imports it
pub trait TenantRouting {
    fn credential_headers(
        &self,
        dynamic_params: Option<&StandardCallbackDynamicParams>,
    ) -> BTreeMap<String, String>;
    fn project_headers(
        &self,
        auth_metadata: Option<&BTreeMap<String, String>>,
    ) -> BTreeMap<String, String>;
    fn endpoint(&self, dynamic_params: Option<&StandardCallbackDynamicParams>) -> Option<String>;
    #[must_use]
    fn routed_config(
        &self,
        credential_headers: &BTreeMap<String, String>,
        project_headers: &BTreeMap<String, String>,
        endpoint: Option<&str>,
        service_name: Option<&str>,
    ) -> Settings;
}

impl TenantRouting for TenantTracerCache {
    // LiteLLM's _credential_headers: a callback owning only a header-ignoring exporter has nowhere to stamp credentials, so it warns once
    fn credential_headers(
        &self,
        dynamic_params: Option<&StandardCallbackDynamicParams>,
    ) -> BTreeMap<String, String> {
        let empty = Json::Null;
        let requested = dynamic_otlp_headers(
            self.callback_name.as_deref(),
            dynamic_params.unwrap_or(&empty),
            &self.allowed_hosts,
        )
        .unwrap_or_default();
        if self.credential_routable || requested.is_empty() {
            return requested;
        }
        unroutable(
            self,
            &self.warned_credential_unroutable,
            "OTel V2: request carries dynamic credentials, but the callback owns no OTLP exporter to stamp them onto; spans export to the default backend",
        )
    }

    // LiteLLM's _project_headers: the project header route is HTTP-only, so a gRPC-only exporter warns once and stays in the default project
    fn project_headers(
        &self,
        auth_metadata: Option<&BTreeMap<String, String>>,
    ) -> BTreeMap<String, String> {
        let requested = project_routing_headers(self.callback_name.as_deref(), auth_metadata);
        if self.project_routable || requested.is_empty() {
            return requested;
        }
        unroutable(
            self,
            &self.warned_project_unroutable,
            "OTel V2: key/team config names a per-request project, but its exporter is not OTLP/HTTP and the project header is HTTP-only; spans stay in the default project",
        )
    }

    fn endpoint(&self, dynamic_params: Option<&StandardCallbackDynamicParams>) -> Option<String> {
        let empty = Json::Null;
        dynamic_otlp_endpoint(
            self.callback_name.as_deref(),
            dynamic_params.unwrap_or(&empty),
            &self.allowed_hosts,
        )
    }

    // LiteLLM's _routed_config: only the callback's own exporters are rewritten, and the tenant's service name wins
    fn routed_config(
        &self,
        credential_headers: &BTreeMap<String, String>,
        project_headers: &BTreeMap<String, String>,
        endpoint: Option<&str>,
        service_name: Option<&str>,
    ) -> Settings {
        let mut routed = Settings {
            exporters: self
                .config
                .exporters
                .iter()
                .map(|spec| {
                    routed_exporter(self, spec, credential_headers, project_headers, endpoint)
                })
                .collect(),
            ..self.config.clone()
        };
        routed.langfuse_span_scope = SpanScope::Full;
        if let Some(name) = service_name {
            name.clone_into(&mut routed.service_name);
        }
        routed
    }
}

// LiteLLM warns once when a request's headers have nowhere to be stamped, then exports it unrouted
fn unroutable(
    cache: &TenantTracerCache,
    warned: &AtomicBool,
    message: &str,
) -> BTreeMap<String, String> {
    if !warned.swap(true, Ordering::Relaxed) {
        tracing::warn!(
            callback = cache.callback_name.as_deref().unwrap_or_default(),
            "{message}"
        );
    }
    BTreeMap::new()
}

// LiteLLM's _routed_exporter: a request carrying one tenant's key must never rewrite a co-configured exporter's headers, which would leak that key
fn routed_exporter(
    cache: &TenantTracerCache,
    spec: &ExporterSpec,
    credential_headers: &BTreeMap<String, String>,
    project_headers: &BTreeMap<String, String>,
    endpoint: Option<&str>,
) -> ExporterSpec {
    let kind = spec.kind.to_lowercase();
    if !owns(spec, cache.callback_name.as_deref()) || NON_OTLP_KINDS.contains(&kind.as_str()) {
        return spec.clone();
    }
    let base = if credential_headers.is_empty() {
        spec.headers.clone()
    } else {
        Some(join(credential_headers, false))
    };
    let headers = if project_headers.is_empty() || GRPC_KINDS.contains(&kind.as_str()) {
        base
    } else {
        let encoded = join(project_headers, true);
        match base.as_deref().filter(|current| !current.is_empty()) {
            Some(existing) => Some(format!("{existing},{encoded}")),
            None => Some(encoded),
        }
    };
    let mut routed = spec.clone();
    if headers != spec.headers {
        routed.headers = headers;
    }
    if let Some(replacement) = endpoint.filter(|value| Some(*value) != spec.endpoint.as_deref()) {
        routed.endpoint = Some(replacement.to_owned());
    }
    routed
}

// LiteLLM compares the callback's name with the exporter's owner, with no owner meaning the unnamed callback owns it
#[must_use]
pub(super) fn owns(spec: &ExporterSpec, callback_name: Option<&str>) -> bool {
    match (spec.owner, callback_name) {
        (None, None) => true,
        (Some(owner), Some(name)) => OWNER_NAMES
            .iter()
            .any(|(known, candidate)| *candidate == owner && *known == name),
        (None, Some(_)) | (Some(_), None) => false,
    }
}

const OWNER_NAMES: [(&str, ExporterOwner); 8] = [
    ("arize", ExporterOwner::ArizeAx),
    ("arize_phoenix", ExporterOwner::ArizePhoenix),
    ("langfuse_otel", ExporterOwner::LangfuseOtel),
    ("weave_otel", ExporterOwner::WeaveOtel),
    ("levo", ExporterOwner::Levo),
    ("agentops", ExporterOwner::Agentops),
    ("newrelic", ExporterOwner::Newrelic),
    ("signoz", ExporterOwner::Signoz),
];

// litellm.integrations.otel.plumbing.routing._plain_header_string and _encoded_header_string: a value containing a comma survives percent-encoding, and parse_env_headers decodes it back
fn join(headers: &BTreeMap<String, String>, encode: bool) -> String {
    headers
        .iter()
        .map(|(key, value)| {
            if encode {
                format!("{key}={}", utf8_percent_encode(value, UNRESERVED))
            } else {
                format!("{key}={value}")
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}
