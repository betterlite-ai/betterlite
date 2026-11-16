mod source;
mod steps;
mod view;

use betterlite_core::api::messages_chat_output::records::{ChoiceRecord, UsageRecord};
use betterlite_core::api::messages_context_clear::result::{
    AppliedEdit, CompactionBlock, UsageIteration,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prompts::arguments::json::Object;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

// One wrapper per case, built from the polyfill output and the inert stream a case names.
#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    logging: bool,
    model: String,
    #[serde(default)]
    tool_name_mapping: Option<IndexMap<String, String>>,
    #[serde(default)]
    applied_edits: Option<Vec<AppliedEdit>>,
    #[serde(default)]
    compaction_block: Option<CompactionBlock>,
    #[serde(default)]
    iterations_usage: Option<Vec<UsageIteration>>,
    #[serde(default)]
    chunks_attr: Option<Vec<StreamChunk>>,
    #[serde(default)]
    messages_attr: Option<Vec<Json>>,
    steps: Vec<Step>,
}

// Every method under test is a step, and the answer pair carries the state the step left behind.
#[derive(Debug, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub(crate) enum Step {
    Snapshot,
    HoldStop {
        event: Object,
    },
    InitialUsage,
    MergeHeldUsage {
        usage: Box<UsageRecord>,
    },
    Choiceless {
        #[serde(default)]
        usage: Option<Box<UsageRecord>>,
    },
    Augment {
        event: Object,
    },
    RefusalStop {
        event: Object,
    },
    Compaction {
        calls: usize,
    },
    DeltaHasContent {
        event: Object,
    },
    IsBlank {
        choices: Vec<ChoiceRecord>,
    },
    StartBlock {
        choices: Vec<ChoiceRecord>,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut harness = steps::Harness::build(input)?;
        let mut answers = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            let answer = steps::answer(&mut harness, step)?;
            let state = view::snapshot(&harness.state, harness.logged_id.as_deref());
            answers.push(json!({"answer": answer, "state": state}));
        }
        Ok(Value::Array(answers))
    }
}
