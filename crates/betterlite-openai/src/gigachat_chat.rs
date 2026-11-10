pub mod config;
pub mod error;
pub mod headers;
pub mod messages;
pub mod options;
pub mod reply;
pub mod request;

mod tests;

// LiteLLM strips this prefix from the model name it sends, wherever the name carries it
pub const MODEL_PREFIX: &str = "gigachat/";
