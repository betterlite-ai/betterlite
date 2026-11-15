pub mod decode;
pub mod error;
pub mod request;
pub mod types;

mod tests;

pub const SUPPORTED_PARAMS: [&str; 3] = ["language", "response_format", "timestamp_granularities"];

pub const SUPPORTS_SUBTITLE_SYNTHESIS: bool = true;
