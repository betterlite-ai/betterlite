use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

// The oracle runs one Azure media operation per input; the case tag names the arm the Python adapter takes.
#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub(crate) case: Kind,
    pub(crate) api_base: String,
    pub(crate) api_version: String,
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) prompt: String,
    pub(crate) api_key: String,
    #[serde(default)]
    pub(crate) options: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) reply: Value,
    #[serde(default, rename = "input")]
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) voice: Option<Json>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    ImageGeneration,
    ImageEdit,
    #[serde(rename = "openai_speech")]
    OpenAiSpeech,
    NativeSsml,
    Transcription,
}
