use betterlite_context::agentic_loop_plan::JsonMap;

const INTERCEPTION_PREFIX: &str = "_websearch_interception";
const LOGGING_KEY: &str = "litellm_logging_obj";

#[must_use]
pub fn prepare_followup_kwargs(kwargs: &JsonMap) -> JsonMap {
    kwargs
        .iter()
        .filter(|(key, _value)| key.as_str() != LOGGING_KEY)
        .filter(|(key, _value)| !key.starts_with(INTERCEPTION_PREFIX))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
