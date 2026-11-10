use crate::otel::config::destination::SpanScope;
use crate::otel::fan_out::attach::{SinkKey, sink_key};
use crate::otel::fan_out::attributes::{in_scope, scoped};
use crate::otel::fan_out::scope::{RequestOtelScope, RequestScopeSource};
use opentelemetry::Context;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{Span, SpanData, SpanProcessor};
use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::sync::Arc;
use std::time::Duration;
pub struct OverriddenBackendFilter {
    pub inner: Box<dyn SpanProcessor>,
    pub owner: Option<String>,
    pub scope: SpanScope,
    pub sink: Option<SinkKey>,
    pub scope_source: Arc<dyn RequestScopeSource>,
}

impl Debug for OverriddenBackendFilter {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("OverriddenBackendFilter")
            .field("owner", &self.owner)
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl OverriddenBackendFilter {
    fn account_scope(&self, request: &RequestOtelScope) -> SpanScope {
        if self.scope == SpanScope::Full {
            return SpanScope::Full;
        }
        let Some(sink) = self.sink.as_ref() else {
            return self.scope;
        };
        let widened = request.destinations.iter().any(|destination| {
            let fields = destination.as_ref();
            fields.span_scope == SpanScope::Full
                && sink_key(Some(fields.endpoint.as_str()), &fields.headers).as_ref() == Some(sink)
        });
        if widened { SpanScope::Full } else { self.scope }
    }
}

impl SpanProcessor for OverriddenBackendFilter {
    fn on_start(&self, span: &mut Span, cx: &Context) {
        self.inner.on_start(span, cx);
    }

    fn on_end(&self, span: SpanData) {
        let request = self.scope_source.scope();
        let suppressed = self
            .owner
            .as_deref()
            .is_some_and(|owner| request.is_suppressed(owner));
        if suppressed || !in_scope(&span, self.scope) {
            return;
        }
        let account_scope = self.account_scope(&request);
        self.inner.on_end(scoped(span, account_scope));
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.inner.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}

#[path = "overridden/tests.rs"]
mod tests;
