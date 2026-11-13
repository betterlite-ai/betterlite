pub const SUPPORTED_OPENAI_PARAMS: [&str; 23] = [
    "frequency_penalty",
    "logit_bias",
    "logprobs",
    "top_logprobs",
    "max_tokens",
    "max_completion_tokens",
    "prediction",
    "n",
    "presence_penalty",
    "seed",
    "stop",
    "stream",
    "stream_options",
    "temperature",
    "top_p",
    "tools",
    "tool_choice",
    "function_call",
    "functions",
    "extra_headers",
    "parallel_tool_calls",
    "response_format",
    "timeout",
];

#[must_use]
pub fn get_supported_openai_params(model: &str) -> Vec<&'static str> {
    let mut params = SUPPORTED_OPENAI_PARAMS.to_vec();
    let without_response_format = model.starts_with("amazon")
        || model.starts_with("cohere")
        || model.starts_with("alephalpha")
        || model == "gpt-4";
    if without_response_format {
        params.retain(|name| *name != "response_format");
    }
    let without_tool_choice = model.starts_with("gemini") || model.starts_with("amazon");
    if without_tool_choice {
        params.retain(|name| *name != "tool_choice");
    }
    params
}
