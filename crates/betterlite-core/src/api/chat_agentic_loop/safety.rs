use betterlite_context::agentic_loop_plan::JsonMap;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::from_str;
use thiserror::Error;

mod tests;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SafetyError {
    #[error("Agentic loop detected repeated tool-call fingerprint; aborting rerun")]
    RepeatedFingerprint,
    #[error("Exceeded max_agentic_loops={max_loops} for model={model}")]
    MaxLoopsExceeded { max_loops: u32, model: String },
    #[error("could not render the agentic loop fingerprint: {0}")]
    Fingerprint(String),
}

pub(crate) fn check_loop_safety(
    tool_calls: &JsonMap,
    fingerprints: &[String],
    depth: u32,
    max_loops: u32,
    model: &str,
) -> Result<String, SafetyError> {
    let fingerprint = fingerprint_tools(tool_calls)?;
    if fingerprints.contains(&fingerprint) {
        return Err(SafetyError::RepeatedFingerprint);
    }
    if depth >= max_loops {
        return Err(SafetyError::MaxLoopsExceeded {
            max_loops,
            model: model.to_owned(),
        });
    }
    Ok(fingerprint)
}

// json.dumps(..., sort_keys=True, default=str); serde_json owns the escaping and the number text
fn fingerprint_tools(tool_calls: &JsonMap) -> Result<String, SafetyError> {
    let fields = tool_calls
        .iter()
        .map(|(key, value)| {
            // the text fallback is json.dumps(default=str) for a value JSON cannot hold, which RawValue never is
            (
                key.clone(),
                from_str::<Json>(value.get())
                    .unwrap_or_else(|_parse_error| Json::Text(value.get().to_owned())),
            )
        })
        .collect();
    serde_json::to_string(&sorted_json(&Json::Object(fields)))
        .map_err(|error| SafetyError::Fingerprint(error.to_string()))
}

// json.dumps(sort_keys=True) sorts every nested object too, so the same call set fingerprints the same however it was built
pub(crate) fn sorted_json(value: &Json) -> Json {
    match value {
        Json::Object(fields) => {
            let mut sorted: IndexMap<String, Json> = fields
                .iter()
                .map(|(key, field)| (key.clone(), sorted_json(field)))
                .collect();
            sorted.sort_keys();
            Json::Object(sorted)
        }
        Json::List(items) => Json::List(items.iter().map(sorted_json).collect()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => value.clone(),
    }
}
