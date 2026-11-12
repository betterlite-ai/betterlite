pub mod chat;
pub mod chunk;
pub mod embed;
pub mod error;

mod tests;

pub const PROVIDER: &str = "cometapi";
pub const DEFAULT_API_BASE: &str = "https://api.cometapi.com/v1";
pub const CHAT_ENDPOINT: &str = "chat/completions";
pub const EMBEDDING_ENDPOINT: &str = "embeddings";
pub const CHUNK_OBJECT: &str = "chat.completion.chunk";
pub const JSON_CONTENT_TYPE: &str = "application/json";
