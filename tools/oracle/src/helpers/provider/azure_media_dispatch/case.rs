use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

// The oracle runs one Azure media operation per input; the case tag names the arm the Python adapter takes.
#[derive(Debug, Deserialize)]
pub(super) struct Input {
    pub(super) case: Kind,
    pub(super) api_base: String,
    pub(super) api_version: String,
    pub(super) model: String,
    #[serde(default)]
    pub(super) prompt: String,
    pub(super) api_key: String,
    #[serde(default)]
    pub(super) options: IndexMap<String, Json>,
    #[serde(default)]
    pub(super) reply: Value,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Kind {
    ImageGeneration,
    ImageEdit,
    Transcription,
}
