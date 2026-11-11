use std::collections::BTreeMap;
use std::mem;

use betterlite_logging::otel::config::destination::OtelDestination;
use betterlite_logging::otel::plumbing::context::context_slots::{
    AnchoredSpan, McpMessageScope, McpTransportSpan, RequestTraceContext,
};
use opentelemetry::Context;
use opentelemetry::trace::TraceContextExt as _;
use rmcp::model::MetaObject;

mod tests;

// the W3C trace context SEP-414 carries in params._meta is all the MCP span may link to
#[must_use]
pub fn meta_carrier(meta: Option<&MetaObject>) -> Option<BTreeMap<String, String>> {
    let source = meta?;
    let mut carrier = BTreeMap::new();
    if let Some(traceparent) = source.get_traceparent() {
        carrier.insert("traceparent".to_owned(), traceparent.to_owned());
    }
    if let Some(tracestate) = source.get_tracestate() {
        carrier.insert("tracestate".to_owned(), tracestate.to_owned());
    }
    (!carrier.is_empty()).then_some(carrier)
}

// what one message carries over from the HTTP request that delivered it, held in that message's own envelope
#[derive(Clone, Default)]
pub struct MessageTrace {
    // the live request context, kept so a failure can still stamp the request's recording span
    pub context: Option<Context>,
    // the transport span of the POST carrying this message; None means the request published none
    pub transport: Option<McpTransportSpan>,
    // None leaves the operation's destinations alone; Some(empty) clears what it inherited
    pub destinations: Option<Vec<OtelDestination>>,
}

impl MessageTrace {
    #[must_use]
    pub fn capture(
        context: Option<&Context>,
        http_route: Option<&str>,
        destinations: Option<Vec<OtelDestination>>,
    ) -> Self {
        let transport = context.and_then(|live| transport_of(live, http_route));
        Self {
            context: context.cloned(),
            transport,
            destinations,
        }
    }
}

// litellm's _otel_publish_transport_span_on_scope stores the request root span, which this message parents under
fn transport_of(context: &Context, http_route: Option<&str>) -> Option<McpTransportSpan> {
    let span = context.span();
    let span_context = span.span_context();
    if !span_context.is_valid() {
        return None;
    }
    Some(McpTransportSpan {
        span: AnchoredSpan {
            context: span_context.clone(),
            http_route: http_route.map(str::to_owned),
        },
        recording: span.is_recording(),
    })
}

// installs one message's carrier, transport and destinations on the operation's trace for as long as it lives
pub struct MessageTraceScope<'trace> {
    trace: &'trace mut RequestTraceContext,
    previous: McpMessageScope,
}

impl<'trace> MessageTraceScope<'trace> {
    #[must_use]
    pub fn enter(
        trace: &'trace mut RequestTraceContext,
        carrier: Option<BTreeMap<String, String>>,
        message: Option<&MessageTrace>,
    ) -> Self {
        let transport = message.and_then(|snapshot| snapshot.transport.clone());
        // LiteLLM's setter returns early on None, so an absent span keeps the slot without re-anchoring the root
        let inherited = if transport.is_none() {
            trace.mcp_transport.clone()
        } else {
            None
        };
        let previous = trace.enter_mcp_message(carrier, transport);
        if let Some(slot) = inherited {
            trace.mcp_transport = Some(slot);
        }
        if let Some(destinations) = message.and_then(|snapshot| snapshot.destinations.as_ref()) {
            trace.destinations.clone_from(destinations);
        }
        Self { trace, previous }
    }
}

impl AsRef<RequestTraceContext> for MessageTraceScope<'_> {
    fn as_ref(&self) -> &RequestTraceContext {
        self.trace
    }
}

// the root anchor deliberately stays on the last recording transport, as B3847 specifies
impl Drop for MessageTraceScope<'_> {
    fn drop(&mut self) {
        self.trace.exit_mcp_message(mem::take(&mut self.previous));
    }
}
