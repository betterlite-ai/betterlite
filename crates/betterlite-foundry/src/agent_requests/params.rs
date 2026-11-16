use betterlite_prompts::arguments::json::Object;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 1] = ["stream"];

#[must_use]
pub const fn map_openai_params(
    _non_default_params: &Object,
    optional_params: Object,
    _model: &str,
    _drop_params: bool,
) -> Object {
    // LiteLLM's agent map hands optional_params back untouched, so no non-default parameter is copied in
    optional_params
}
