pub mod cleaner;
pub mod decode;
pub mod error;
pub mod studio;
mod tests;
pub mod transport;
pub mod vertex;

// both count adapters answer as gemini, even the Vertex subclass that LiteLLM inherits
pub const PROVIDER: &str = "gemini";
