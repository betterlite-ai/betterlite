// LiteLLM's get_supported_openai_params for text completion: the fixed list, in its order.
pub const SUPPORTED_OPENAI_PARAMS: [&str; 13] = [
    "max_tokens",
    "logprobs",
    "echo",
    "temperature",
    "top_p",
    "top_k",
    "frequency_penalty",
    "presence_penalty",
    "n",
    "stop",
    "response_format",
    "stream",
    "user",
];
