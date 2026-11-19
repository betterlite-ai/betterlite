pub mod beta;
pub mod body;
pub mod environment;
pub mod error;

mod tests;

// LiteLLM's Vertex native Messages config reports this identity whatever the caller passed.
pub const PROVIDER: &str = "vertex_ai";
