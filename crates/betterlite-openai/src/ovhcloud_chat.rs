pub mod chunk;
pub mod error;
pub mod request;
mod tests;

pub const PROVIDER: &str = "ovhcloud";
pub const DEFAULT_API_BASE: &str = "https://oai.endpoints.kepler.ai.cloud.ovh.net/v1";
pub const CHAT_ENDPOINT: &str = "chat/completions";
pub const CHUNK_OBJECT: &str = "chat.completion.chunk";
pub const JSON_CONTENT_TYPE: &str = "application/json";
