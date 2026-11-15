pub mod error;
mod fields;
pub mod params;
pub mod reply;
pub mod request;
pub mod stream;
mod tests;

// LiteLLM answers every should_fake_stream call with false and has no native websocket
pub const SHOULD_FAKE_STREAM: bool = false;
pub const SUPPORTS_NATIVE_WEBSOCKET: bool = false;
