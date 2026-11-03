use super::wire::callback::Callback;
use crate::otel::model::trace_controls::TraceControls;

#[must_use]
pub fn caller_session_id(kwargs: &Callback, trace: &TraceControls) -> Option<String> {
    let bodies = [
        &kwargs.litellm_params.metadata,
        &kwargs.litellm_params.litellm_metadata,
    ];
    let minted: Vec<String> = bodies
        .iter()
        .filter_map(|body| {
            body.litellm_session_id_generated
                .filter(|generated| *generated)?;
            body.session_id.clone()
        })
        .collect();
    if !minted.is_empty() {
        return trace
            .session_id
            .clone()
            .filter(|session| !minted.contains(session))
            .or_else(|| {
                bodies
                    .iter()
                    .filter_map(|body| body.session_id.clone())
                    .find(|session| !minted.contains(session))
            });
    }
    let explicit = kwargs.litellm_params.litellm_session_id.clone();
    let echoes_trace = explicit.as_ref().is_some_and(|session| {
        bodies
            .iter()
            .any(|body| body.trace_id.as_ref() == Some(session))
    });
    explicit
        .filter(|_| !echoes_trace)
        .or_else(|| trace.session_id.clone())
}

mod tests;
