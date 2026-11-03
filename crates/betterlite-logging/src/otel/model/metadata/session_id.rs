use super::common::{field, object, string};
use crate::otel::model::trace_controls::TraceControls;
use crate::otel::model::utils::coercion::OtelModelValue;
use std::collections::BTreeMap;

#[must_use]
pub fn caller_session_id(kwargs: &OtelModelValue, trace: &TraceControls) -> Option<String> {
    let params = object(field(kwargs, "litellm_params"));
    let bodies: Vec<&BTreeMap<String, OtelModelValue>> = ["metadata", "litellm_metadata"]
        .into_iter()
        .filter_map(|key| {
            let values = params?;
            object(values.get(key))
        })
        .collect();
    let minted: Vec<String> = bodies
        .iter()
        .filter_map(|body| {
            let generated = body.get("litellm_session_id_generated");
            generated.filter(|marker| marker.as_bool() == Some(true))?;
            string(body.get("session_id"))
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
                    .filter_map(|body| string(body.get("session_id")))
                    .find(|session| !minted.contains(session))
            });
    }
    let explicit = params.and_then(|values| string(values.get("litellm_session_id")));
    let echoes_trace = explicit.as_ref().is_some_and(|session| {
        bodies
            .iter()
            .any(|body| string(body.get("trace_id")).as_ref() == Some(session))
    });
    explicit
        .filter(|_| !echoes_trace)
        .or_else(|| trace.session_id.clone())
}

mod tests;
