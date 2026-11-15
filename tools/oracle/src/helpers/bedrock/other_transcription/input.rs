use betterlite_bedrock::transcription::params::Params;
use betterlite_openai::batches::sdk::TimeoutPhases;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Calls {
    #[serde(default)]
    pub(crate) calls: Vec<Call>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    Sync,
    Async,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum File {
    Bytes { content: String },
    Tuple { filename: String, content: String },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    pub(crate) mode: Mode,
    pub(crate) model: String,
    pub(crate) file: File,
    pub(crate) custom_llm_provider: String,
    pub(crate) optional_params: Params,
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) extra_headers: Option<Params>,
    pub(crate) timeout: Option<f64>,
    pub(crate) phases: Option<TimeoutPhases>,
    pub(crate) reply_text: Option<String>,
}
