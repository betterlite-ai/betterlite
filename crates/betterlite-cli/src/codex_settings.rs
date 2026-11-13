pub mod canonical;
pub mod configure;
pub mod document;
pub mod error;
pub mod paths;
pub mod preflight;
pub mod receipt;
pub mod snapshot;
pub mod storage;
pub mod unconfigure;
pub mod version;

pub const CODEX_PROXY_PROVIDER: &str = "litellm";
pub const MODEL_PROVIDERS_KEY: &str = "model_providers";
pub const OWNED_PATHS: [&str; 4] = [
    "model_provider",
    "model",
    "profile",
    "model_providers.litellm",
];

mod tests;
