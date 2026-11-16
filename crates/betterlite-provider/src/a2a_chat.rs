pub mod auth;
pub mod error;
pub mod records;
pub mod registry;
pub mod reply;
pub mod request;
pub mod stream;

mod tests;

// LiteLLM's default depth for the A2A text extraction
pub const MAX_TEXT_DEPTH: usize = 10;
