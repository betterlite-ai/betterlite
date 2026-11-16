use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_core::api::messages_chat_output::error::ChatOutputError;
use betterlite_core::api::messages_chat_output::output::ContentBlock;
use betterlite_core::api::messages_chat_output::records::{
    ChoiceRecord, ResponseRecord, TerminalRecord, UsageRecord,
};
use betterlite_core::api::messages_chat_output::response::response_message;
use betterlite_core::api::messages_chat_output::stream::{content_block, delta, terminal_frame};
use betterlite_core::api::messages_context_clear::result::{
    AppliedEdit, CompactionBlock, PolyfillResult, UsageIteration,
};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Response {
        response: Box<ResponseRecord>,
        #[serde(default)]
        tool_name_mapping: Option<Box<IndexMap<String, String>>>,
        #[serde(default)]
        polyfill_result: Option<Box<PolyfillInput>>,
    },
    Block {
        choices: Vec<ChoiceRecord>,
    },
    Delta {
        choices: Vec<ChoiceRecord>,
    },
    Terminal {
        #[serde(default)]
        choices: Vec<ChoiceRecord>,
        #[serde(default)]
        usage: Option<Box<UsageRecord>>,
        #[serde(default)]
        hidden_usage: Option<Box<UsageRecord>>,
        #[serde(default)]
        applied_edits: Vec<AppliedEdit>,
        #[serde(default)]
        index: i64,
    },
}

// Only the polyfill fields an output method reads travel from a case.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PolyfillInput {
    #[serde(default)]
    applied_edits: Vec<AppliedEdit>,
    #[serde(default)]
    compaction_block: Option<CompactionBlock>,
    #[serde(default)]
    iterations_usage: Option<Vec<UsageIteration>>,
}

impl PolyfillInput {
    fn to_result(&self) -> PolyfillResult {
        PolyfillResult {
            messages: Vec::new(),
            system: None,
            applied_edits: self.applied_edits.clone(),
            compaction_block: self.compaction_block.clone(),
            iterations_usage: self.iterations_usage.clone(),
        }
    }
}

// B198's seam, fixed so a generated tool id is comparable.
struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(1)
    }
}

// Each failure carries the exception the pinned adapter raises on the same input.
fn failure(error: &ChatOutputError) -> Box<HelperError> {
    let class = match error {
        ChatOutputError::Arguments { .. } => "ValueError",
        ChatOutputError::NonObjectArguments { .. } => "ValidationError",
        ChatOutputError::NonTextThinkingField { .. } => "AssertionError",
        ChatOutputError::NoChoices => "IndexError",
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

const fn block_kind(block: &ContentBlock) -> &'static str {
    match block {
        ContentBlock::Text { .. } | ContentBlock::RedactedThinking { .. } => "text",
        ContentBlock::Thinking { .. } => "thinking",
        ContentBlock::ToolUse { .. } => "tool_use",
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Response {
                response,
                tool_name_mapping,
                polyfill_result,
            } => {
                let polyfill = polyfill_result.as_deref().map(PolyfillInput::to_result);
                let answer =
                    response_message(response, tool_name_mapping.as_deref(), polyfill.as_ref())
                        .map_err(|error| failure(&error))?;
                Ok(serde_json::to_value(answer)?)
            }
            Input::Block { choices } => {
                let block = content_block(choices, &FixedIds).map_err(|error| failure(&error))?;
                Ok(json!({"block_type": block_kind(&block), "block": block}))
            }
            Input::Delta { choices } => {
                let answer = delta(choices).map_err(|error| failure(&error))?;
                Ok(serde_json::to_value(answer)?)
            }
            Input::Terminal {
                choices,
                usage,
                hidden_usage,
                applied_edits,
                index,
            } => {
                let record = TerminalRecord {
                    choices: choices.clone(),
                    usage: usage.as_deref().cloned(),
                    hidden_usage: hidden_usage.as_deref().cloned(),
                    index: *index,
                    applied_edits: applied_edits.clone(),
                };
                let answer = terminal_frame(&record).map_err(|error| failure(&error))?;
                Ok(serde_json::to_value(answer)?)
            }
        }
    }
}
