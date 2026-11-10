use crate::otel::config::destination::{OtelDestination, SpanScope};
use crate::otel::fan_out::attach::{SinkKey, sink_key};
use crate::otel::fan_out::attributes::{
    attribute_map, in_scope, is_excluded_database_span, scoped, tenant_view,
};
use crate::otel::fan_out::destination::DestinationProcessorFactory;
use crate::otel::fan_out::drain::{DRAIN_WORKERS, DrainPool};
use crate::otel::fan_out::scope::{RequestOtelScope, RequestScopeSource};
use indexmap::IndexMap;
use opentelemetry::Context;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::trace::{Span, SpanData, SpanProcessor};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};
mod cache;
use cache::{CacheState, CachedProcessor};
#[derive(Clone)]
pub struct FanOutSettings {
    pub operator_sinks: BTreeMap<SinkKey, SpanScope>,
    pub excluded_db_systems: BTreeSet<String>,
    pub shutdown_drain: Duration,
    pub pending_drains: usize,
    pub drain_pool: Option<Arc<DrainPool>>,
}

impl Debug for FanOutSettings {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("FanOutSettings")
            .field("pending_drains", &self.pending_drains)
            .finish_non_exhaustive()
    }
}

impl Debug for FanOutInner {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("FanOutInner")
            .field("shutdown_drain", &self.shutdown_drain)
            .finish_non_exhaustive()
    }
}

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
        let mut failed = false;
        for processor in self.inner.snapshot() {
            failed |= processor.force_flush().is_err();
        }
        if failed {
            return Err(OTelSdkError::InternalFailure(
                "a destination processor did not flush".to_owned(),
            ));
        }
        Ok(())
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown(self.inner.shutdown_drain.min(timeout));
        Ok(())
    }

    fn set_resource(&mut self, resource: &Resource) {
        *self
            .inner
            .resource
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = resource.clone();
    }
}

impl FanOutInner {
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

    // Shutdown closes nothing under a span: in-flight exports finish first, stragglers are retired.
    fn shutdown(&self, timeout: Duration) {
        let deadline = Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(Instant::now);
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
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
                closing.push(Arc::clone(&processor.processor));
            }
        }
        cache.processors.clear();
        cache.retired = kept;
        drop(cache);
        for processor in closing {
            self.drain.submit(processor);
        }
        self.drain
            .close(deadline.saturating_duration_since(Instant::now()));
    }

    fn snapshot(&self) -> Vec<Arc<dyn SpanProcessor>> {
        let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
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
