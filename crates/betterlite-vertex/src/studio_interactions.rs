pub mod credentials;
pub mod decode;
pub mod error;
pub mod records;
pub mod request;
pub mod urls;

mod tests;

pub const API_VERSION: &str = "v1beta";
pub const API_REVISION: &str = "2026-05-20";

pub const SUPPORTED_PARAMS: [&str; 14] = [
    "model",
    "agent",
    "input",
    "tools",
    "system_instruction",
    "generation_config",
    "stream",
    "store",
    "background",
    "environment",
    "response_modalities",
    "response_format",
    "response_mime_type",
    "previous_interaction_id",
];
