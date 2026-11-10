use opentelemetry::trace::SpanKind;

use crate::otel::vocabulary::registry::{LiteLLMSpanKind, SPAN_REGISTRY, SpanRole};

// litellm.integrations.otel.plumbing.providers._SPAN_KIND_BY_ROLE_KIND
#[must_use]
pub const fn to_otel_span_kind(kind: LiteLLMSpanKind) -> SpanKind {
    match kind {
        LiteLLMSpanKind::Server => SpanKind::Server,
        LiteLLMSpanKind::Client => SpanKind::Client,
        LiteLLMSpanKind::Internal => SpanKind::Internal,
        LiteLLMSpanKind::Producer => SpanKind::Producer,
        LiteLLMSpanKind::Consumer => SpanKind::Consumer,
    }
}

// the registry is validated to cover every role, so the fallback never fires in practice
#[must_use]
pub fn span_kind(role: SpanRole) -> SpanKind {
    let mut kind = SpanKind::Internal;
    for spec in &SPAN_REGISTRY {
        if spec.role == role {
            kind = to_otel_span_kind(spec.kind);
        }
    }
    kind
}
