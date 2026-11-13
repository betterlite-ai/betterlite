pub mod capabilities;
pub mod error;
pub mod lyria;
pub mod partners;
pub mod publisher;
pub mod route;

mod tests;

// publishers accept these on a self-deployed endpoint; LiteLLM strips them before the call
pub const UNSUPPORTED_DEPLOYED_PARAMS: [&str; 10] = [
    "audio",
    "max_retries",
    "modalities",
    "prediction",
    "prompt_cache_key",
    "prompt_cache_retention",
    "safety_identifier",
    "service_tier",
    "store",
    "web_search_options",
];
