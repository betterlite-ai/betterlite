use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::vertex_speech::mapping::SpeechVoice;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Mapped { cases: Vec<MappingCase> },
    Prepared { cases: Vec<PreparedCase> },
    Decoded { cases: Vec<DecodedCase> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MappingCase {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) params: Object,
    pub(crate) voice: Option<SpeechVoice>,
    #[serde(default)]
    pub(crate) drop_params: bool,
    #[serde(default)]
    pub(crate) global_drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreparedCase {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) params: Object,
    pub(crate) voice: Option<SpeechVoice>,
    #[serde(default)]
    pub(crate) drop_params: bool,
    #[serde(default)]
    pub(crate) global_drop_params: bool,
    pub(crate) project: Option<String>,
    pub(crate) location: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) input: String,
    #[serde(default)]
    pub(crate) headers: IndexMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DecodedCase {
    pub(crate) model: String,
    pub(crate) status: u16,
    pub(crate) body: String,
}
