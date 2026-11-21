use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use opentelemetry::Context;
use opentelemetry::KeyValue;
use opentelemetry::trace::TraceContextExt as _;

use crate::otel::fan_out::scope::{RequestOtelScope, RequestScopeSource};
use crate::otel::langfuse_root::{RootSpanSink, RootSpanSource};
use crate::otel::logger::v2::RequestTraceSource;
use crate::otel::plumbing::context::context_slots::RequestTraceContext;
use crate::otel::plumbing::context::destination_routing::destinations_are_additive;

use super::slot::ACTIVE_REQUEST_SLOT;

// litellm.integrations.otel.plumbing.context: the three ports read the request task's slot, and the additive switch is fixed once from the environment
#[derive(Debug)]
pub struct ProxyTraceSources {
    additive: bool,
}

impl ProxyTraceSources {
    #[must_use]
    pub fn new(environment: &dyn EnvironmentLookup) -> Self {
        Self {
            additive: destinations_are_additive(None, environment),
        }
    }
}

impl RequestTraceSource for ProxyTraceSources {
    fn trace_context(&self) -> RequestTraceContext {
        ACTIVE_REQUEST_SLOT
            .try_with(Arc::clone)
            .ok()
            .map_or_else(RequestTraceContext::default, |slot| slot.trace().clone())
    }
}

impl RequestScopeSource for ProxyTraceSources {
    fn scope(&self) -> RequestOtelScope {
        RequestOtelScope {
            destinations: ACTIVE_REQUEST_SLOT
                .try_with(Arc::clone)
                .ok()
                .map_or_else(Vec::new, |slot| slot.trace().destinations.clone()),
            additive: self.additive,
        }
    }
}

impl RootSpanSource for ProxyTraceSources {
    fn request_root_span(&self) -> Option<Arc<dyn RootSpanSink>> {
        let slot = ACTIVE_REQUEST_SLOT.try_with(Arc::clone).ok()?;
        let live = slot.live_root().clone();
        if !live.span().span_context().is_valid() {
            return None;
        }
        Some(Arc::new(LiveRootSpan { context: live }))
    }
}

// the sink keeps the Context, not a copy of the span, so Langfuse's root stamping and the layer that ends the span write the same span
struct LiveRootSpan {
    context: Context,
}

impl RootSpanSink for LiveRootSpan {
    fn is_recording(&self) -> bool {
        self.context.span().is_recording()
    }

    fn set_attribute(&self, key: &str, value: String) {
        self.context
            .span()
            .set_attribute(KeyValue::new(key.to_owned(), value));
    }
}
