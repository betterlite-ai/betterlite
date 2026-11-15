pub mod auth;
pub mod error;
pub mod params;
pub mod response;
pub mod tokens;

mod tests;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 2] = ["language", "response_format"];

pub const SONIOX_PASSTHROUGH_PARAMS: [&str; 12] = [
    "language_hints",
    "language_hints_strict",
    "enable_language_identification",
    "enable_speaker_diarization",
    "context",
    "translation",
    "client_reference_id",
    "webhook_url",
    "webhook_auth_header_name",
    "webhook_auth_header_value",
    "audio_url",
    "file_id",
];

pub const SONIOX_HANDLER_ONLY_PARAMS: [&str; 4] = [
    "soniox_polling_interval",
    "soniox_max_polling_attempts",
    "soniox_cleanup",
    "filename",
];

pub const SONIOX_DEFAULT_POLL_INTERVAL: f64 = 1.0;
pub const SONIOX_MIN_POLL_INTERVAL: f64 = 0.5;
pub const SONIOX_MAX_POLL_INTERVAL: f64 = 60.0;
pub const SONIOX_DEFAULT_ATTEMPTS: i64 = 1800;
pub const SONIOX_MAX_ATTEMPTS: i64 = 6000;
pub const SONIOX_DEFAULT_CLEANUP: [&str; 2] = ["file", "transcription"];
