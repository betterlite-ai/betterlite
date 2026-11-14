pub mod endpointing;
pub mod error;
pub mod grpc_status;
pub mod options;
pub mod recognition;
pub mod response;

mod tests;

use betterlite_openai::openai_base_config::Headers;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 3] =
    ["language", "response_format", "timestamp_granularities"];

#[derive(Debug, Clone, Copy, Default)]
pub struct RivaAudioTranscriptionConfig;

impl RivaAudioTranscriptionConfig {
    pub fn validate_environment(headers: Headers) -> Headers {
        // gRPC auth is built in the call layer, so the source returns the headers untouched
        headers
    }
}
