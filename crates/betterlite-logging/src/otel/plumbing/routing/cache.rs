use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};

use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_models::prompt_contracts::StandardCallbackDynamicParams;
use indexmap::IndexMap;
use opentelemetry::InstrumentationScope;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::{SdkTracer, SdkTracerProvider};

use crate::otel::config::Settings;
use crate::otel::config::destination::OtelDestination;
use crate::otel::fan_out::scope::RequestScopeSource;
use crate::otel::plumbing::context::destination_routing::destination_backends;
use crate::otel::plumbing::exporters::{ExportResult, ExporterFactories, Transport};
use crate::otel::plumbing::routing::routed::{RouteKey, owns, tenant_service_name};
use crate::otel_provider_assembly::tracer_provider_assembly::{
    AssemblyOptions, build_tracer_provider,
};

const SCOPE_VERSION: &str = env!("CARGO_PKG_VERSION");

// LiteLLM's _MAX_CACHED_PROVIDERS: the LRU bound on distinct routed providers, below which an
// unbounded cache would let a caller spawn one provider (and exporter thread) per credential set
pub const MAX_CACHED_PROVIDERS: usize = 256;

// LiteLLM's _MAX_RETIRED_PROVIDERS (a quarter of the cache cap): evicted providers kept draining
// their open spans, bounded so their threads can't accumulate past the cache cap
pub const MAX_RETIRED_PROVIDERS: usize = 64;

// the tracer to create a span on, plus whether it must root its own trace
pub struct TenantRoute {
    pub tracer: SdkTracer,
    pub detached: bool,
    pub provider: Option<Arc<TenantProvider>>,
}

// a routed provider plus the identity eviction and release count their leases by
pub struct TenantProvider {
    pub id: u64,
    pub provider: SdkTracerProvider,
}

#[derive(Debug, Default)]
struct CacheState {
    providers: IndexMap<RouteKey, Arc<TenantProvider>>,
    // the open spans each provider still carries, so eviction can't shut it down under them
    open_span_counts: HashMap<u64, usize>,
    // evicted providers draining their open spans, oldest first so an overflow sheds the stalest
    retired: IndexMap<u64, Arc<TenantProvider>>,
    next_id: u64,
}

// litellm.integrations.otel.plumbing.routing.TenantTracerCache
pub struct TenantTracerCache {
    pub(super) config: Settings,
    pub(super) callback_name: Option<String>,
    pub(super) allowed_hosts: Arc<[AllowedDestination]>,
    pub(super) project_routable: bool,
    pub(super) credential_routable: bool,
    pub(super) warned_project_unroutable: AtomicBool,
    pub(super) warned_credential_unroutable: AtomicBool,
    factories: Arc<ExporterFactories>,
    scope: InstrumentationScope,
    scope_source: Arc<dyn RequestScopeSource>,
    state: Mutex<CacheState>,
}

impl TenantTracerCache {
    #[must_use]
    pub fn new(
        config: Settings,
        callback_name: Option<String>,
        tracer_name: &str,
        factories: Arc<ExporterFactories>,
        scope_source: Arc<dyn RequestScopeSource>,
        allowed_hosts: Arc<[AllowedDestination]>,
    ) -> Self {
        let transports: Vec<Transport> = config
            .exporters
            .iter()
            .filter(|spec| owns(spec, callback_name.as_deref()))
            .map(|spec| factories.exporter_transport(&spec.kind))
            .collect();
        Self {
            config,
            callback_name,
            allowed_hosts,
            project_routable: transports.contains(&Transport::Http),
            credential_routable: transports
                .iter()
                .any(|transport| matches!(transport, Transport::Http | Transport::Grpc)),
            warned_project_unroutable: AtomicBool::new(false),
            warned_credential_unroutable: AtomicBool::new(false),
            factories,
            scope: InstrumentationScope::builder(tracer_name.to_owned())
                .with_version(SCOPE_VERSION)
                .build(),
            scope_source,
            state: Mutex::new(CacheState::default()),
        }
    }

    // litellm.integrations.otel.plumbing.routing.TenantTracerCache.release
    pub fn release(&self, provider: Option<&Arc<TenantProvider>>) {
        let Some(provider) = provider else {
            return;
        };
        let drained = {
            let mut state = self.lock();
            let remaining = state
                .open_span_counts
                .get(&provider.id)
                .copied()
                .unwrap_or(0)
                .saturating_sub(1);
            if remaining > 0 {
                state.open_span_counts.insert(provider.id, remaining);
                return;
            }
            state.open_span_counts.remove(&provider.id);
            state.retired.shift_remove(&provider.id).is_some()
        };
        if drained {
            shutdown_provider(&provider.provider);
        }
    }

    // litellm.integrations.otel.plumbing.routing.TenantTracerCache.route_for
    pub fn route_for(
        &self,
        default: &SdkTracer,
        dynamic_params: Option<&StandardCallbackDynamicParams>,
        auth_metadata: Option<&BTreeMap<String, String>>,
        destinations: &[OtelDestination],
    ) -> ExportResult<TenantRoute> {
        let backends = destination_backends(destinations);
        if self
            .callback_name
            .as_ref()
            .is_some_and(|name| backends.contains(name))
        {
            return Ok(default_route(default));
        }
        let credential_headers = self.credential_headers(dynamic_params);
        let project_headers = self.project_headers(auth_metadata);
        let service_name = tenant_service_name(auth_metadata);
        let tenant_account = !credential_headers.is_empty() || !project_headers.is_empty();
        if !tenant_account && (service_name.is_none() || !backends.is_empty()) {
            return Ok(default_route(default));
        }
        let endpoint = self.endpoint(dynamic_params);
        let key = RouteKey::new(
            &credential_headers,
            &project_headers,
            endpoint.as_deref(),
            service_name.as_deref(),
        );
        let (provider, evicted) = {
            let mut state = self.lock();
            let built = self.cached_provider(
                &mut state,
                &key,
                &credential_headers,
                &project_headers,
                endpoint.as_deref(),
                service_name.as_deref(),
            )?;
            let count = state.open_span_counts.entry(built.id).or_insert(0);
            *count = count.saturating_add(1);
            let evicted = evict_overflow(&mut state);
            (built, evicted)
        };
        if let Some(evicted) = evicted {
            shutdown_provider(&evicted);
        }
        Ok(TenantRoute {
            tracer: provider.provider.tracer_with_scope(self.scope.clone()),
            detached: tenant_account,
            provider: Some(provider),
        })
    }

    // LiteLLM's _cached_provider_locked: an LRU hit moves to the recent end, a miss builds a provider
    fn cached_provider(
        &self,
        state: &mut CacheState,
        key: &RouteKey,
        credential_headers: &BTreeMap<String, String>,
        project_headers: &BTreeMap<String, String>,
        endpoint: Option<&str>,
        service_name: Option<&str>,
    ) -> ExportResult<Arc<TenantProvider>> {
        if let Some(cached) = state.providers.shift_remove(key) {
            state.providers.insert(key.clone(), Arc::clone(&cached));
            return Ok(cached);
        }
        let config =
            self.routed_config(credential_headers, project_headers, endpoint, service_name);
        let provider = Arc::new(TenantProvider {
            id: state.next_id,
            provider: build_tracer_provider(
                &config,
                &self.factories,
                &self.scope_source,
                AssemblyOptions::default(),
            )?,
        });
        state.next_id = state.next_id.saturating_add(1);
        state.providers.insert(key.clone(), Arc::clone(&provider));
        Ok(provider)
    }

    fn lock(&self) -> MutexGuard<'_, CacheState> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

// the unrouted answer: the logger's own tracer, parented into the request trace as usual
fn default_route(default: &SdkTracer) -> TenantRoute {
    TenantRoute {
        tracer: default.clone(),
        detached: false,
        provider: None,
    }
}

// LiteLLM's _evicted_on_overflow_locked: a provider with open spans retires to drain instead of
// being shut down under them, and the stalest retiree is shut down once too many pile up
fn evict_overflow(state: &mut CacheState) -> Option<SdkTracerProvider> {
    if state.providers.len() <= MAX_CACHED_PROVIDERS {
        return None;
    }
    let (_, evicted) = state.providers.shift_remove_index(0)?;
    if state
        .open_span_counts
        .get(&evicted.id)
        .copied()
        .unwrap_or(0)
        == 0
    {
        return Some(evicted.provider.clone());
    }
    state.retired.insert(evicted.id, evicted);
    if state.retired.len() <= MAX_RETIRED_PROVIDERS {
        return None;
    }
    let (overflowed, retired) = state.retired.shift_remove_index(0)?;
    state.open_span_counts.remove(&overflowed);
    Some(retired.provider.clone())
}

// the shutdown runs outside the lock: it force-flushes over the network and must not stall routing
fn shutdown_provider(provider: &SdkTracerProvider) {
    if let Err(error) = provider.shutdown() {
        tracing::debug!(%error, "OTel V2: error shutting down evicted provider");
    }
}
