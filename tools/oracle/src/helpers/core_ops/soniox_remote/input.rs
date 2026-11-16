use std::collections::BTreeMap;

use betterlite_openai::openai_base_config::Headers;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Call {
    Upload(UploadArgs),
    Poll(PollArgs),
    Cleanup(CleanupArgs),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Answer {
    pub(super) status: u16,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: String,
    #[serde(default)]
    pub(super) fail: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct UploadArgs {
    pub(super) auth_headers: Headers,
    pub(super) api_base: String,
    pub(super) filename: String,
    pub(super) content: String,
    pub(super) content_type: String,
    pub(super) filename_override: Option<String>,
    pub(super) timeout_ms: u64,
    pub(super) answers: Vec<Answer>,
}

#[derive(Debug, Deserialize)]
pub(super) struct PollArgs {
    pub(super) auth_headers: Headers,
    pub(super) api_base: String,
    pub(super) transcription_id: String,
    pub(super) poll_interval: f64,
    pub(super) max_attempts: i64,
    pub(super) timeout_ms: u64,
    pub(super) answers: Vec<Answer>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CleanupArgs {
    pub(super) auth_headers: Headers,
    pub(super) api_base: String,
    pub(super) cleanup: Vec<String>,
    pub(super) transcription_id: Option<String>,
    pub(super) file_id: Option<String>,
    pub(super) timeout_ms: u64,
    pub(super) answers: Vec<Answer>,
}
