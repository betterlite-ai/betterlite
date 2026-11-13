pub mod reply;
pub mod request;
mod tests;

pub const TRANSCRIPTION_ENDPOINT: &str = "audio/transcriptions";
pub const MAX_REPLY_BYTES: usize =
    betterlite_http::proxy::limits::TransportLimits::DEFAULT.body_bytes;
