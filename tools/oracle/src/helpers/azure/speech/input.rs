use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

// Each case file names the production entry it drives; the seven files share four entry shapes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    AudioSpeech(AudioSpeech),
    SupportedParams(AudioSpeech),
    NativeSpeechSelection(NativeSelection),
    Probe(ProbeCases),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AudioSpeech {
    pub(crate) model: String,
    pub(crate) input: String,
    pub(crate) voice: String,
    #[serde(default)]
    pub(crate) optional_params: IndexMap<String, Json>,
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: String,
    pub(crate) api_version: String,
    #[serde(default)]
    pub(crate) aspeech: bool,
    pub(crate) reply: Reply,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeSelection {
    pub(crate) calls: Vec<NativeCall>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeCall {
    pub(crate) model: String,
    pub(crate) input: String,
    #[serde(default)]
    pub(crate) voice: Option<Json>,
    #[serde(default)]
    pub(crate) optional_params: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) litellm_params: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    pub(crate) timeout_seconds: u64,
    #[serde(default)]
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) aspeech: bool,
    #[serde(default)]
    pub(crate) globals: Globals,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Globals {
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) azure_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProbeCases {
    pub(crate) scenarios: Vec<ProbeScenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProbeScenario {
    pub(crate) outcome: Outcome,
    #[serde(default)]
    pub(crate) model: Option<String>,
    pub(crate) api_key: String,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) api_version: Option<String>,
    #[serde(default)]
    pub(crate) mode: ProbeModeInput,
    #[serde(default)]
    pub(crate) messages: Option<Json>,
    #[serde(default)]
    pub(crate) reply: Option<Reply>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProbeModeInput {
    #[default]
    Chat,
    ImageGeneration,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    Request,
    Reply,
    InvalidCompletion,
}
