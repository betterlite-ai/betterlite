pub const OPENAI_TRANSCRIPTION_PARAMS: &[&str] =
    &["language", "response_format", "timestamp_granularities"];

pub const OPENAI_EMBEDDING_PARAMS: &str = "dimensions encoding_format user";

pub const OPENAI_CHAT_COMPLETION_PARAMS: &str = "functions function_call temperature top_p n \
    stream stream_options stop max_completion_tokens modalities prediction audio max_tokens \
    presence_penalty frequency_penalty logit_bias user request_timeout api_base api_version \
    api_key deployment_id organization base_url default_headers timeout response_format seed \
    tools tool_choice max_retries parallel_tool_calls logprobs top_logprobs reasoning_effort \
    extra_headers thinking web_search_options include_server_side_tool_invocations service_tier \
    prompt_cache_key prompt_cache_retention safety_identifier verbosity store";

pub const DEFAULT_CHAT_COMPLETION_PARAMS: &str = "functions function_call temperature top_p n \
    stream stream_options stop max_tokens max_completion_tokens modalities prediction audio \
    presence_penalty frequency_penalty logit_bias user model custom_llm_provider response_format \
    seed tools tool_choice max_retries logprobs top_logprobs extra_headers api_version \
    parallel_tool_calls drop_params allowed_openai_params additional_drop_params messages \
    reasoning_effort verbosity thinking web_search_options include_server_side_tool_invocations \
    service_tier safety_identifier prompt_cache_key prompt_cache_retention store metadata \
    context_management";

pub const PROVIDER_UNVALIDATED_PARAMS: &[&str] =
    &["user", "stream_options", "stream", "max_retries"];

pub const EXTRA_BODY_ROUTING_KEYS: &[&str] = &["model"];

pub const EXTRA_BODY_PROVIDERS: &[&str] = &["openai", "azure", "text-completion-openai"];

const OPENAI_COMPATIBLE_PROVIDERS: &str = "anyscale groq nvidia_nim cerebras baseten sambanova \
    ai21_chat ai21 volcengine codestral deepseek tencent deepinfra perplexity xinference xai zai \
    together_ai fireworks_ai empower friendliai azure_ai github litellm_proxy hosted_vllm \
    llamafile lm_studio galadriel github_copilot chatgpt novita meta_llama publicai synthetic \
    tensormesh apertis nano-gpt poe chutes parasail libertai featherless_ai nscale nebius \
    dashscope qwencloud qwen_ai_platform modelscope moonshot v0 helicone morph lambda_ai \
    inception hyperbolic vercel_ai_gateway aiml edenai wandb cometapi clarifai \
    docker_model_runner ragflow pinstripes darkbloom meta cognition cortecs scx-ai prism sail";

#[must_use]
pub fn is_openai_compatible_provider(name: &str) -> bool {
    OPENAI_COMPATIBLE_PROVIDERS
        .split_whitespace()
        .any(|provider| provider == name)
}
