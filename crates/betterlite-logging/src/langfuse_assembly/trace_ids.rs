use betterlite_models::decisions::Json;

use crate::langfuse_projections::session_header::is_session_header_trace;
use crate::langfuse_projections::text::optional_str;

use super::GenerationTrace;
use super::metadata::Steering;

pub struct CallTrace {
    pub trace_id: Option<String>,
    pub call_trace_id: Option<String>,
    pub existing: Option<String>,
}

// litellm.integrations.langfuse.langfuse.LangFuseLogger._log_langfuse_v2 trace id resolution
#[must_use]
pub fn resolve(trace: &GenerationTrace<'_>, steering: &Steering) -> CallTrace {
    let existing = optional_str(steering.existing_trace_id.as_ref());
    let trace_id = existing
        .clone()
        .or_else(|| requested_trace_id(trace, steering));
    let call_trace_id = call_id(trace, steering, trace_id.as_deref());
    CallTrace {
        trace_id,
        call_trace_id,
        existing,
    }
}

// metadata first, then the standard payload, then the call's own ids
fn requested_trace_id(trace: &GenerationTrace<'_>, steering: &Steering) -> Option<String> {
    optional_str(steering.trace_id.as_ref())
        .or_else(|| trace.payload.map(|payload| payload.trace_id.clone()))
        .or_else(|| trace.litellm_trace_id.map(str::to_owned))
        .or_else(|| trace.litellm_call_id.map(str::to_owned))
}

// a session header names the session, not this call, so the call id keeps each generation trace apart
fn call_id(
    trace: &GenerationTrace<'_>,
    steering: &Steering,
    trace_id: Option<&str>,
) -> Option<String> {
    if steering.existing_trace_id.is_some() || !session_header(trace, steering, trace_id) {
        return trace_id.map(str::to_owned);
    }
    trace
        .litellm_call_id
        .map(str::to_owned)
        .or_else(|| trace_id.map(str::to_owned))
}

fn session_header(
    trace: &GenerationTrace<'_>,
    steering: &Steering,
    trace_id: Option<&str>,
) -> bool {
    let trace_value = trace_id.map(|id| Json::Text(id.to_owned()));
    is_session_header_trace(
        trace_value.as_ref(),
        steering.session_id.as_ref(),
        super::proxy_request(trace.litellm_params),
    )
}
