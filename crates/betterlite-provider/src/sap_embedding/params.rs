use betterlite_prompts::arguments::json::Object;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 1] = ["encoding_format"];

#[must_use]
pub fn get_supported_openai_params(model: &str) -> Vec<&'static str> {
    if model.contains("text-embedding-3") {
        vec!["encoding_format", "dimensions"]
    } else {
        SUPPORTED_OPENAI_PARAMS.to_vec()
    }
}

#[must_use]
pub const fn map_openai_params(
    _non_default_params: &Object,
    optional_params: Object,
    _model: &str,
    _drop_params: bool,
) -> Object {
    // LiteLLM's embedding map hands optional_params back untouched, so encoding_format and dimensions never reach the body
    optional_params
}
