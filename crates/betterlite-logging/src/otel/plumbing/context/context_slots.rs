use std::collections::BTreeMap;
use std::mem;

use opentelemetry::Context;
use opentelemetry::trace::{SpanContext, TraceContextExt as _};

use crate::otel::config::destination::OtelDestination;

// the request's root SERVER span, captured once so children parent to it explicitly, never ambient
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchoredSpan {
    pub context: SpanContext,
    pub http_route: Option<String>,
}

// the transport span of the HTTP request carrying the current MCP message
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpTransportSpan {
    pub span: AnchoredSpan,
    pub recording: bool,
}

// what one MCP message replaced, handed back so the session task can restore it afterwards
#[derive(Debug, Clone, Default)]
pub struct McpMessageScope {
    carrier: Option<BTreeMap<String, String>>,
    transport: Option<McpTransportSpan>,
    destinations: Vec<OtelDestination>,
}

#[derive(Debug, Clone, Default)]
pub struct RequestTraceContext {
    pub root: Option<AnchoredSpan>,
    pub mcp_carrier: Option<BTreeMap<String, String>>,
    pub mcp_transport: Option<McpTransportSpan>,
    pub destinations: Vec<OtelDestination>,
}

impl RequestTraceContext {
    // a non-recordable span never replaces a good anchor, and the anchor is idempotent by design
    pub fn anchor_request_root(&mut self, span: AnchoredSpan) {
        if span.context.is_valid() {
            self.root = Some(span);
        }
    }

    // children of the request parent to the anchor; with none they take whatever is ambient
    #[must_use]
    pub fn resolve_request_span_context(&self) -> Context {
        self.root
            .as_ref()
            .filter(|root| root.context.is_valid())
            .map_or_else(Context::current, |root| {
                Context::current().with_remote_span_context(root.context.clone())
            })
    }

    // litellm.integrations.otel.plumbing.context.set_mcp_message_transport_span and its carrier sibling
    pub fn enter_mcp_message(
        &mut self,
        carrier: Option<BTreeMap<String, String>>,
        transport: Option<McpTransportSpan>,
    ) -> McpMessageScope {
        let usable = transport.filter(|candidate| candidate.span.context.is_valid());
        if let Some(open) = usable.as_ref().filter(|candidate| candidate.recording) {
            self.root = Some(open.span.clone());
        }
        McpMessageScope {
            carrier: mem::replace(&mut self.mcp_carrier, carrier),
            transport: mem::replace(&mut self.mcp_transport, usable),
            destinations: self.destinations.clone(),
        }
    }

    // the root anchor deliberately stays on the finished message, as LiteLLM's contextvar is never reset
    pub fn exit_mcp_message(&mut self, scope: McpMessageScope) {
        self.mcp_carrier = scope.carrier;
        self.mcp_transport = scope.transport;
        self.destinations = scope.destinations;
    }

    // only a transport still open for writes can receive an error stamp from another task
    #[must_use]
    pub const fn mcp_message_transport_span(&self) -> Option<&SpanContext> {
        match self.mcp_transport.as_ref() {
            Some(transport) if transport.recording => Some(&transport.span.context),
            Some(_) | None => None,
        }
    }
}

#[path = "context_slots/tests.rs"]
mod tests;
