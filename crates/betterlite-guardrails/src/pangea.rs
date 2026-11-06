pub mod config;
pub mod error;
pub mod hooks;
pub mod wire;

#[path = "pangea/tests/config_model.rs"]
mod config_model;
#[path = "pangea/tests/environment.rs"]
mod environment;
#[path = "pangea/tests/missing_api_key.rs"]
mod missing_api_key;
mod tests;
#[path = "pangea/tests/text_completion.rs"]
mod text_completion;
