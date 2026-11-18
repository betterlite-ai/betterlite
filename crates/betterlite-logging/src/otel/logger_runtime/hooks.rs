use opentelemetry::Context;

use crate::otel::model::payloads::error::SpanError;
use crate::otel::plumbing::context::context_slots::RequestTraceContext;
use crate::otel::v2_callback::identity::IdentitySeed;
use crate::otel::v2_callback::phase::PhaseSpan;
use crate::otel::v2_callback::registered;
use crate::otel::v2_callback::registry::V2Registry;

use super::LoggerRuntime;

// litellm.integrations.otel.runtime: the optional handle a proxy supplies around its registered logger
pub trait RuntimeHooks: Send + Sync {
    fn phase_span(&self, name: &str, trace: &RequestTraceContext) -> Option<PhaseSpan>;

    fn seed_request_identity(
        &self,
        seed: &IdentitySeed<'_>,
        trace: &mut RequestTraceContext,
    ) -> Option<Context>;
}

// runtime.py's _otel_runtime resolves to proxy_server.open_telemetry_logger, which B3853 reaches the same way
impl RuntimeHooks for V2Registry {
    fn phase_span(&self, name: &str, trace: &RequestTraceContext) -> Option<PhaseSpan> {
        registered::phase_span(self, name, trace)
    }

    fn seed_request_identity(
        &self,
        seed: &IdentitySeed<'_>,
        trace: &mut RequestTraceContext,
    ) -> Option<Context> {
        registered::seed_request_identity(self, seed, trace)
    }
}

// runtime.py's phase_span context manager: no hook or no logger runs the body with no span
pub fn phase_span<T, F>(
    runtime: &LoggerRuntime,
    name: &str,
    trace: &RequestTraceContext,
    body: F,
) -> Result<T, SpanError>
where
    F: FnOnce(Option<&PhaseSpan>) -> Result<T, SpanError>,
{
    let Some(phase) = runtime
        .hooks
        .as_ref()
        .and_then(|hooks| hooks.phase_span(name, trace))
    else {
        return body(None);
    };
    let _guard = phase.attach();
    body(Some(&phase)).inspect_err(|error| phase.record_error(error))
}

// runtime.py's seed_request_identity returns nothing; B3853's Context is inert when no hook is present
pub fn seed_request_identity(
    runtime: &LoggerRuntime,
    seed: &IdentitySeed<'_>,
    trace: &mut RequestTraceContext,
) -> Option<Context> {
    if let Some(hooks) = runtime.hooks.as_ref() {
        return hooks.seed_request_identity(seed, trace);
    }
    None
}
