use betterlite_prompts::arguments::json::Object;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 10] = [
    "stream",
    "temperature",
    "max_tokens",
    "top_p",
    "stop",
    "seed",
    "tools",
    "tool_choice",
    "functions",
    "function_call",
];

// a non-default parameter LiteLLM advertises but never maps is dropped here without a warning, as the source does
pub fn map_openai_params(non_default: &Object, optional_params: &mut Object, model: &str) {
    for (param, value) in non_default {
        match param.as_str() {
            "stream" => {
                optional_params.insert(String::from("stream"), value.clone());
            }
            "max_tokens" => {
                let target = if model.contains("vicuna") || model.contains("flan") {
                    "max_length"
                } else if model.contains("meta/codellama-13b") {
                    "max_tokens"
                } else {
                    "max_new_tokens"
                };
                optional_params.insert(String::from(target), value.clone());
            }
            "temperature" => {
                optional_params.insert(String::from("temperature"), value.clone());
            }
            "top_p" => {
                optional_params.insert(String::from("top_p"), value.clone());
            }
            "stop" => {
                optional_params.insert(String::from("stop_sequences"), value.clone());
            }
            _ => {}
        }
    }
}

mod tests;
