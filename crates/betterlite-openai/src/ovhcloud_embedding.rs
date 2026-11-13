pub mod reply;
pub mod request;
mod tests;

pub const EMBEDDING_ENDPOINT: &str = "embeddings";
pub const MAX_REPLY_BYTES: usize =
    betterlite_http::proxy::limits::TransportLimits::DEFAULT.body_bytes;
