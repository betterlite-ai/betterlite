// LiteLLM's generic realtime connection: _open_realtime_backend_ws and async_realtime, with the dialer and relay injected
pub mod dial;
pub mod error;
pub mod protocol;
pub mod session;

mod tests;
