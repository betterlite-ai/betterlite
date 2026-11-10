use std::fmt::Debug;

use crate::otel::config::destination::OtelDestination;

// litellm.integrations.otel.plumbing.context.request_destinations and tenant_destinations_are_additive as one request view
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestOtelScope {
    pub destinations: Vec<OtelDestination>,
    pub additive: bool,
}

impl RequestOtelScope {
    // litellm.integrations.otel.plumbing.context.suppressed_backends
    #[must_use]
    pub fn is_suppressed(&self, callback_name: &str) -> bool {
        !self.additive
            && self.destinations.iter().any(|destination| {
                destination.as_ref().callback_name.as_deref() == Some(callback_name)
            })
    }
}

// The request's destinations and mode as a span processor reads them; the proxy owns where they live.
pub trait RequestScopeSource: Send + Sync + Debug {
    fn scope(&self) -> RequestOtelScope;
}

// request_destinations outside a proxy request: the empty default of LiteLLM's ContextVar
#[derive(Debug, Clone, Copy, Default)]
pub struct NoRequestScope;

impl RequestScopeSource for NoRequestScope {
    fn scope(&self) -> RequestOtelScope {
        RequestOtelScope::default()
    }
}

#[path = "scope/tests.rs"]
mod tests;
