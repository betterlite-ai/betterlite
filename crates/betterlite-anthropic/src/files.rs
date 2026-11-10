pub mod api;
pub mod error;
pub mod request;
mod tests;
pub mod wire;
pub const DEFAULT_PURPOSE: &str = "messages";
pub const FILES_BETA_HEADER: &str = "files-api-2025-04-14";
pub const MESSAGE_BATCH_ID_PREFIX: &str = "msgbatch_";
pub const SUPPORTED_OPENAI_PARAMS: [&str; 1] = ["purpose"];
