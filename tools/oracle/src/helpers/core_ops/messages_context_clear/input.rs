use betterlite_core::api::messages_context_clear::result::{AppliedEdit, CompactionBlock};
use betterlite_core::api::messages_context_clear::spec::ClearToolUsesEditSpec;
use betterlite_models::decisions::Json;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum Input {
    Editor(Box<EditorInput>),
    VisibleEdits(VisibleEditsInput),
    Constants,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditorInput {
    pub(crate) model: String,
    pub(crate) messages: Vec<Json>,
    #[serde(default)]
    pub(crate) tools: Option<Vec<Json>>,
    #[serde(default)]
    pub(crate) system: Option<Json>,
    pub(crate) edit_spec: ClearToolUsesEditSpec,
    pub(crate) counter: CounterInput,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(crate) enum CounterInput {
    Scripted { values: Vec<usize> },
    Real,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VisibleEditsInput {
    #[serde(default)]
    pub(crate) applied_edits: Vec<AppliedEdit>,
    #[serde(default)]
    pub(crate) compaction_block: Option<CompactionBlock>,
    #[serde(default)]
    pub(crate) error: Option<ContextErrorInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContextErrorInput {
    pub(crate) status_code: u16,
    pub(crate) message: String,
}
