use std::collections::BTreeMap;

use opentelemetry::Context;

use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::context::context_slots::RequestTraceContext;
use crate::otel::v2_callback::identity::IdentitySeed;
use crate::otel::v2_callback::phase::PhaseSpan;
use crate::otel::v2_callback::registry::V2Registry;

// litellm.integrations.otel.logger.emit_guardrail_span, one entry in and one span out through the published logger
pub fn emit_guardrail_span(
    registry: &V2Registry,
    entry: &BTreeMap<String, OtelModelValue>,
    trace: &RequestTraceContext,
) {
    let Some(logger) = registry.registered_v2_logger() else {
        return;
    };
    logger.emit_guardrail_span(entry, trace);
}

// litellm.integrations.otel.logger.seed_request_identity
#[must_use]
pub fn seed_request_identity(
    registry: &V2Registry,
    seed: &IdentitySeed<'_>,
    trace: &mut RequestTraceContext,
    context: &Context,
) -> Option<Context> {
    let logger = registry.registered_v2_logger()?;
    let seeded = logger.seed_request_identity(seed, trace, context);
    Some(seeded)
}

// litellm.integrations.otel.logger.phase_span, None without a logger so callers check the span
#[must_use]
pub fn phase_span(registry: &V2Registry, name: &str) -> Option<PhaseSpan> {
    let logger = registry.registered_v2_logger()?;
    let span = logger.start_phase_span(name);
    Some(span)
}
