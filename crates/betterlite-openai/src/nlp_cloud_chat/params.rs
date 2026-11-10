use crate::openai_base_config::Object;

// LiteLLM lists these in get_supported_openai_params and maps them in map_openai_params
pub const SUPPORTED_OPENAI_PARAMS: [&str; 8] = [
    "max_tokens",
    "stream",
    "temperature",
    "top_p",
    "presence_penalty",
    "frequency_penalty",
    "n",
    "stop",
];

#[must_use]
pub fn map_openai_params(non_default_params: &Object, mut optional_params: Object) -> Object {
    for (name, value) in non_default_params {
        let target = match name.as_str() {
            "max_tokens" => "max_length",
            "n" => "num_return_sequences",
            "stop" => "stop_sequences",
            "stream" | "temperature" | "top_p" | "presence_penalty" | "frequency_penalty" => {
                name.as_str()
            }
            _ => continue,
        };
        optional_params.insert(target.to_owned(), value.clone());
    }
    optional_params
}
