use betterlite_core::api::messages_context_clear::result::CompactionBlock;
use betterlite_core::api::summary_history::system::SystemPrompt;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum Input {
    Slicing(SlicingInput),
    Trigger(TriggerInput),
    Prompts(Box<PromptsInput>),
    Metadata(MetadataInput),
    Count(Box<CountInput>),
    Extraction(ExtractionInput),
    ClientHistory(ClientHistoryInput),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SlicingInput {
    pub(crate) messages: Vec<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TriggerInput {
    pub(crate) edit_spec: Object,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    Translated,
    Broken,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PromptsInput {
    pub(crate) edit_spec: Object,
    pub(crate) tools: Option<Vec<Json>>,
    pub(crate) system: Option<SystemPrompt>,
    pub(crate) summary: Option<String>,
    pub(crate) messages: Vec<Json>,
    pub(crate) translation: Outcome,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MetadataInput {
    pub(crate) parent: Option<Object>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(crate) enum CounterInput {
    Scripted { values: Vec<usize> },
    Real,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CountInput {
    pub(crate) model: String,
    pub(crate) messages: Vec<Json>,
    pub(crate) compaction_block: Option<CompactionBlock>,
    pub(crate) tools: Option<Vec<Json>>,
    pub(crate) system: Option<SystemPrompt>,
    pub(crate) counter: CounterInput,
    pub(crate) translation: Outcome,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExtractionInput {
    pub(crate) raw: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClientHistoryInput {
    pub(crate) messages: Vec<Json>,
    pub(crate) system: Option<SystemPrompt>,
}
