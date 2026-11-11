pub const SUPPORTED_OPENAI_PARAMS: [&str; 1] = ["encoding_format"];

#[must_use]
pub fn get_supported_openai_params(model: &str) -> Vec<&'static str> {
    if model.contains("text-embedding-3") {
        vec!["encoding_format", "dimensions"]
    } else {
        vec!["encoding_format"]
    }
}
