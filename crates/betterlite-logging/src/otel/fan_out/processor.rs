use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use indexmap::IndexMap;
use opentelemetry::Context;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::trace::{Span, SpanData, SpanProcessor};

use crate::otel::config::destination::{OtelDestination, SpanScope};
use crate::otel::fan_out::attach::{SinkKey, sink_key};
use crate::otel::fan_out::attributes::{
    attribute_map, in_scope, is_excluded_database_span, scoped, tenant_view,
};
use crate::otel::fan_out::destination::DestinationProcessorFactory;
use crate::otel::fan_out::drain::{DRAIN_WORKERS, DrainPool, MAX_PENDING_DRAINS, SHUTDOWN_DRAIN};
use crate::otel::fan_out::scope::{RequestOtelScope, RequestScopeSource};

mod cache;

use cache::{CacheState, CachedProcessor};

// The fan-out state an operator can tune; a second logger's fan-out gets its own drain pool.
#[derive(Debug, Clone)]
pub struct FanOutSettings {
    pub operator_sinks: BTreeMap<SinkKey, SpanScope>,
    pub excluded_db_systems: BTreeSet<String>,
    pub shutdown_drain: Duration,
    pub pending_drains: usize,
    pub drain_pool: Option<Arc<DrainPool>>,
}

impl Default for FanOutSettings {
    fn default() -> Self {
        Self {
            operator_sinks: BTreeMap::new(),
            excluded_db_systems: BTreeSet::new(),
            shutdown_drain: SHUTDOWN_DRAIN,
            pending_drains: MAX_PENDING_DRAINS,
            drain_pool: None,
        }
    }
}

#[derive(Debug)]
struct FanOutInner {
    operator_sinks: BTreeMap<SinkKey, SpanScope>,
    excluded_db_systems: BTreeSet<String>,
    shutdown_drain: Duration,
    build: Arc<dyn DestinationProcessorFactory>,
    scope: Arc<dyn RequestScopeSource>,
    drain: Arc<DrainPool>,
    cache: Mutex<CacheState>,
    idle: Condvar,
    resource: Mutex<Resource>,
}

// litellm.integrations.otel.plumbing.providers.TenantFanOutSpanProcessor
#[derive(Debug, Clone)]
pub struct TenantFanOutSpanProcessor {
    inner: Arc<FanOutInner>,
}

impl TenantFanOutSpanProcessor {
    #[must_use]
    pub fn new(
        scope: Arc<dyn RequestScopeSource>,
        build: Arc<dyn DestinationProcessorFactory>,
        settings: FanOutSettings,
    ) -> Self {
        let drain = settings
            .drain_pool
            .unwrap_or_else(|| Arc::new(DrainPool::new(DRAIN_WORKERS, settings.pending_drains)));
        Self {
            inner: Arc::new(FanOutInner {
                operator_sinks: settings.operator_sinks,
                excluded_db_systems: settings.excluded_db_systems,
                shutdown_drain: settings.shutdown_drain,
                build,
                scope,
                drain,
                cache: Mutex::new(CacheState::default()),
                idle: Condvar::new(),
                resource: Mutex::new(Resource::builder_empty().build()),
            }),
        }
    }

    // litellm.integrations.otel.plumbing.providers.TenantFanOutSpanProcessor.deliverable
    #[must_use]
    pub fn deliverable(&self, destinations: &[OtelDestination]) -> Vec<OtelDestination> {
        destinations
            .iter()
            .filter(|destination| cache::buildable(&self.inner, destination))
            .cloned()
            .collect()
    }
}

impl SpanProcessor for TenantFanOutSpanProcessor {
    fn on_start(&self, _span: &mut Span, _cx: &Context) {}

    fn on_end(&self, span: SpanData) {
        let request = self.inner.scope.scope();
        if request.destinations.is_empty() {
            return;
        }
        let attributes = attribute_map(&span.attributes);
        for destination in &request.destinations {
            if self
                .inner
                .operator_already_writes(&span, destination, &request)
                || !in_scope(&span, destination.as_ref().span_scope)
                || is_excluded_database_span(&attributes, &self.inner.excluded_db_systems)
            {
                continue;
            }
            let Some(processor) = cache::acquire(&self.inner, destination) else {
                continue;
            };
            let view = scoped(tenant_view(&span), destination.as_ref().span_scope);
            processor.processor.on_end(view);
            cache::release(&self.inner, &processor);
        }
    }

    fn force_flush(&self) -> OTelSdkResult {
        let mut flushed = true;
        for processor in self.inner.snapshot() {
            flushed &= processor.force_flush().is_ok();
        }
        if flushed {
            Ok(())
        } else {
            Err(OTelSdkError::InternalFailure(
                "a destination processor did not flush".to_owned(),
            ))
        }
    }

    // The configured drain bound wins when the SDK allows more time, as LiteLLM's own shutdown does.
    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown(self.inner.shutdown_drain.min(timeout));
        Ok(())
    }

    fn set_resource(&mut self, resource: &Resource) {
        let mut stored = match self.inner.resource.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *stored = resource.clone();
    }
}

fn lock_cache(cache: &Mutex<CacheState>) -> MutexGuard<'_, CacheState> {
    match cache.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl FanOutInner {
    // litellm.integrations.otel.plumbing.providers.TenantFanOutSpanProcessor._operator_already_writes
    fn operator_already_writes(
        &self,
        span: &SpanData,
        destination: &OtelDestination,
        request: &RequestOtelScope,
    ) -> bool {
        let fields = destination.as_ref();
        if fields
            .callback_name
            .as_deref()
            .is_some_and(|name| request.is_suppressed(name))
        {
            return false;
        }
        let Some(sink) = sink_key(Some(fields.endpoint.as_str()), &fields.headers) else {
            return false;
        };
        self.operator_sinks
            .get(&sink)
            .is_some_and(|operator_scope| in_scope(span, *operator_scope))
    }

    // litellm.integrations.otel.plumbing.providers.TenantFanOutSpanProcessor.shutdown
    fn shutdown(&self, timeout: Duration) {
        let deadline = Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(Instant::now);
        let mut cache = lock_cache(&self.cache);
        cache.closed = true;
        while !cache.exporting.is_empty() {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            cache = match self.idle.wait_timeout(cache, remaining) {
                Ok((guard, _timed_out)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
        let live: Vec<Arc<CachedProcessor>> = cache
            .processors
            .values()
            .chain(cache.retired.values())
            .cloned()
            .collect();
        let mut closing = Vec::new();
        let mut kept = IndexMap::new();
        for processor in &live {
            if cache.exporting.contains_key(&processor.id) {
                kept.insert(processor.id, Arc::clone(processor));
            } else {
                closing.push(Arc::clone(processor));
            }
        }
        cache.processors.clear();
        cache.retired = kept;
        drop(cache);
        for processor in closing {
            self.drain.submit(Arc::clone(&processor.processor));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        self.drain.close(Some(remaining));
    }

    fn snapshot(&self) -> Vec<Arc<dyn SpanProcessor>> {
        let cache = lock_cache(&self.cache);
        cache
            .processors
            .values()
            .chain(cache.retired.values())
            .map(|cached| Arc::clone(&cached.processor))
            .collect()
    }
}

#[path = "processor/tests.rs"]
mod tests;
