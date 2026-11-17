use std::collections::BTreeMap;

use opentelemetry::Context;

use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::context::context_slots::RequestTraceContext;
use crate::otel::v2_callback::guardrail;
use crate::otel::v2_callback::identity::{self, IdentitySeed};
use crate::otel::v2_callback::phase::{self, PhaseSpan};
use crate::otel::v2_callback::registry::V2Registry;

// litellm.integrations.otel.logger.emit_guardrail_span, one entry in and one span out through the published logger
pub fn emit_guardrail_span(
    registry: &V2Registry,
    entry: &BTreeMap<String, OtelModelValue>,
    trace: &RequestTraceContext,
) {
    if let Some(logger) = registry.registered_v2_logger() {
        guardrail::emit_guardrail_span(&logger, entry, trace);
    }
}

// litellm.integrations.otel.logger.seed_request_identity
#[must_use]
pub fn seed_request_identity(
    registry: &V2Registry,
    seed: &IdentitySeed<'_>,
    trace: &mut RequestTraceContext,
) -> Option<Context> {
    registry
        .registered_v2_logger()
        .map(|logger| identity::seed_request_identity(&logger, seed, trace))
}

// litellm.integrations.otel.logger.phase_span, None without a logger so callers check the span
#[must_use]
pub fn phase_span(
    registry: &V2Registry,
    name: &str,
    trace: &RequestTraceContext,
) -> Option<PhaseSpan> {
    registry
        .registered_v2_logger()
        .map(|logger| phase::start_phase_span(&logger, name, trace))
}
